export type InlineToken =
  | { kind: "text"; text: string }
  | { kind: "bold"; children: InlineToken[] }
  | { kind: "italic"; text: string }
  | { kind: "code"; text: string }
  | { kind: "link"; label: string; href: string }
  | { kind: "tool-call"; toolName: string; tools: string[]; raw: string };

export interface ResolvedFileTarget {
  path: string;
  line: number | null;
}

export function tokenizeInline(input: string): InlineToken[] {
  const tokens: InlineToken[] = [];
  if (!input) return tokens;

  const regex =
    /(\[Appel d'outils?(?:\s+natifs?)?\s*:[^\]]+\]|\[[^\]]+\]\([^)\s]+\)|`[^`]+`|\*\*[^*]+\*\*|\*[^*]+\*)/gi;
  let lastIdx = 0;
  let match: RegExpExecArray | null;

  while ((match = regex.exec(input)) !== null) {
    if (match.index > lastIdx) {
      tokens.push({ kind: "text", text: input.slice(lastIdx, match.index) });
    }

    const raw = match[0];
    if (raw.startsWith("[")) {
      const toolMatch = /^\[Appel d'outils?(?:\s+natifs?)?\s*:\s*([^\]]+)\]$/i.exec(raw);
      if (toolMatch) {
        const tools = toolMatch[1]
          .split(",")
          .map((t) => t.trim().replace(/^`+|`+$/g, ""))
          .filter(Boolean);
        tokens.push({
          kind: "tool-call",
          toolName: tools[0] || "tool",
          tools,
          raw,
        });
      } else {
        const linkMatch = /^\[([^\]]+)\]\(([^)\s]+)\)$/.exec(raw);
        if (linkMatch) {
          const cleanLabel = linkMatch[1].trim().replace(/^`+|`+$/g, "");
          tokens.push({
            kind: "link",
            label: cleanLabel || linkMatch[2],
            href: linkMatch[2].trim(),
          });
        } else {
          tokens.push({ kind: "text", text: raw });
        }
      }
    } else if (raw.startsWith("`") && raw.endsWith("`")) {
      tokens.push({ kind: "code", text: raw.slice(1, -1) });
    } else if (raw.startsWith("**") && raw.endsWith("**")) {
      tokens.push({ kind: "bold", children: tokenizeInline(raw.slice(2, -2)) });
    } else if (raw.startsWith("*") && raw.endsWith("*")) {
      tokens.push({ kind: "italic", text: raw.slice(1, -1) });
    }

    lastIdx = regex.lastIndex;
  }

  if (lastIdx < input.length) {
    tokens.push({ kind: "text", text: input.slice(lastIdx) });
  }

  return tokens;
}

export function resolveFileLinkTarget(
  rawRef: string,
  availablePaths: string[],
  projectRoot?: string,
  allowFallback = false
): ResolvedFileTarget | null {
  let cleaned = (rawRef || "")
    .trim()
    .replace(/^[`'"]+|[`'"]+$/g, "")
    .trim();

  if (!cleaned) return null;
  if (/^(https?:\/\/|mailto:|#)/i.test(cleaned)) return null;

  let line: number | null = null;
  const hashLineMatch = /#L(\d+)(?:-L?\d+)?$/i.exec(cleaned);
  if (hashLineMatch) {
    const parsed = Number(hashLineMatch[1]);
    if (Number.isFinite(parsed) && parsed > 0) line = parsed;
    cleaned = cleaned.slice(0, hashLineMatch.index);
  } else {
    const colonLineMatch = /:(\d+)$/.exec(cleaned);
    if (colonLineMatch) {
      const parsed = Number(colonLineMatch[1]);
      if (Number.isFinite(parsed) && parsed > 0) line = parsed;
      cleaned = cleaned.slice(0, colonLineMatch.index);
    }
  }

  if (cleaned.startsWith("file://")) {
    cleaned = cleaned.slice("file://".length);
    try {
      cleaned = decodeURIComponent(cleaned);
    } catch {
      // Keep raw string if malformed URI sequence
    }
  }

  cleaned = cleaned.replace(/\\/g, "/");

  if (projectRoot) {
    const normRoot = projectRoot.replace(/\\/g, "/").replace(/\/+$/, "");
    if (normRoot && cleaned.startsWith(normRoot + "/")) {
      cleaned = cleaned.slice(normRoot.length + 1);
    } else if (normRoot && cleaned === normRoot) {
      return null;
    }
  }

  while (cleaned.startsWith("./")) {
    cleaned = cleaned.slice(2);
  }
  cleaned = cleaned.trim();

  if (!cleaned) return null;
  const lower = cleaned.toLowerCase();

  // 1. Exact path
  const exact = availablePaths.find((p) => p === cleaned);
  if (exact) return { path: exact, line };

  // 2. Case-insensitive path
  const ci = availablePaths.find((p) => p.toLowerCase() === lower);
  if (ci) return { path: ci, line };

  // 3. Suffix match (".../cleaned")
  const suffix = availablePaths.find(
    (p) => p.endsWith("/" + cleaned) || p.toLowerCase().endsWith("/" + lower)
  );
  if (suffix) return { path: suffix, line };

  // 4. Unique basename match
  const baseMatches = availablePaths.filter((p) => {
    const base = p.split("/").pop() || p;
    return base.toLowerCase() === lower;
  });
  if (baseMatches.length === 1) {
    return { path: baseMatches[0], line };
  }

  if (
    allowFallback &&
    !cleaned.startsWith("/") &&
    !cleaned.includes("..") &&
    (/^[a-zA-Z0-9_\-./]+\.[a-zA-Z0-9]{1,12}$/.test(cleaned) || cleaned.includes("/"))
  ) {
    return { path: cleaned, line };
  }

  return null;
}
