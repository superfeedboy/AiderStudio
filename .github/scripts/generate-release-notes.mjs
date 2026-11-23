#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const CONVENTIONAL_TYPES = [
  "feat",
  "feature",
  "fix",
  "design",
  "ui",
  "ux",
  "style",
  "tech",
  "refactor",
  "perf",
  "chore",
  "ci",
  "build",
  "docs",
  "test",
  "tests",
  "deps",
  "revert",
  "breaking",
  "security",
  "sec",
  "infra",
];

const SUBJECT_HEADER_RE = new RegExp(
  `^(${CONVENTIONAL_TYPES.join("|")}|release)(?:\\(([^)]+)\\))?(!)?:\\s+(.+)$`,
  "i",
);

// Fallback for arbitrary type with explicit "!" breaking marker on subject (e.g. "api!: ...")
const GENERIC_BREAKING_SUBJECT_RE = /^([a-zA-Z][a-zA-Z0-9_-]*)(?:\(([^)]+)\))?!:\s+(.+)$/;

const BODY_CONVENTIONAL_RE = new RegExp(
  `^(${CONVENTIONAL_TYPES.join("|")})(?:\\(([^)]+)\\))?(!)?:\\s+(.+)$`,
  "i",
);

const BREAKING_FOOTER_RE = /^BREAKING[- ]CHANGE:\s*(.*)$/i;

const DESIGN_TYPES = new Set(["design", "ui", "ux", "style"]);
const DESIGN_SCOPES = new Set([
  "design",
  "ui",
  "ux",
  "css",
  "style",
  "styles",
  "theme",
  "layout",
  "icon",
  "icons",
]);
const FEATURE_TYPES = new Set(["feat", "feature"]);

const REDUNDANT_SCOPES_BY_SECTION = {
  breaking: new Set(["breaking"]),
  features: new Set(["feat", "feature", "features"]),
  design: new Set(["design"]),
  tech: new Set(["tech"]),
};

function runGit(args, { allowFailure = false } = {}) {
  try {
    return execFileSync("git", args, {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    }).trimEnd();
  } catch (err) {
    if (allowFailure) {
      return "";
    }
    throw err;
  }
}

/**
 * Normalize a git remote URL into an HTTPS repository URL.
 * Returns null if origin cannot be resolved.
 */
export function inferRepoUrl(remoteUrl = null) {
  if (!remoteUrl && process.env.GITHUB_REPOSITORY) {
    const server = (process.env.GITHUB_SERVER_URL || "https://github.com").replace(/\/+$/, "");
    return `${server}/${process.env.GITHUB_REPOSITORY.replace(/^\/+|\/+$/g, "")}`;
  }

  const raw = remoteUrl ?? runGit(["remote", "get-url", "origin"], { allowFailure: true });
  if (!raw) {
    return null;
  }

  const trimmed = raw.trim().replace(/\/+$/, "").replace(/\.git$/i, "");

  const scpMatch = trimmed.match(/^git@([^:]+):(.+)$/);
  if (scpMatch) {
    return `https://${scpMatch[1]}/${scpMatch[2].replace(/^\/+/, "")}`;
  }

  const sshMatch = trimmed.match(/^ssh:\/\/(?:[^@]+@)?([^/]+)\/(.+)$/);
  if (sshMatch) {
    return `https://${sshMatch[1]}/${sshMatch[2].replace(/^\/+/, "")}`;
  }

  if (/^https?:\/\//i.test(trimmed)) {
    return trimmed.replace(/^(https?:\/\/)[^@]+@/, "$1");
  }

  return null;
}

/**
 * Find the previous v* tag reachable from currentTag.
 * For stable tags (no "-"), prefers the latest previous non-prerelease tag.
 */
export function findPreviousTag(currentTag) {
  const output = runGit(
    ["-c", "versionsort.suffix=-", "tag", "--merged", currentTag, "--sort=-v:refname", "v*"],
    { allowFailure: true },
  );
  if (!output) {
    return null;
  }

  const tags = output
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);

  if (tags.length === 0) {
    return null;
  }

  const currentIdx = tags.indexOf(currentTag);
  const candidates =
    currentIdx !== -1 ? tags.slice(currentIdx + 1) : tags.filter((tag) => tag !== currentTag);

  if (candidates.length === 0) {
    return null;
  }

  const isCurrentStable = !currentTag.includes("-");
  if (isCurrentStable) {
    const previousStable = candidates.find((tag) => !tag.includes("-"));
    if (previousStable) {
      return previousStable;
    }
  }

  return candidates[0] ?? null;
}

function isReleaseBumpHeader(type, scope) {
  const lowerType = (type || "").toLowerCase();
  const lowerScope = (scope || "").toLowerCase();
  return lowerType === "release" || (lowerType === "chore" && lowerScope === "release");
}

function parseConventionalLine(line, allowGenericBreaking = false) {
  const match = line.match(SUBJECT_HEADER_RE);
  if (match) {
    return {
      type: match[1].toLowerCase(),
      scope: match[2] ? match[2].trim() : null,
      breaking: Boolean(match[3]),
      description: match[4].trim(),
    };
  }

  if (allowGenericBreaking) {
    const genericMatch = line.match(GENERIC_BREAKING_SUBJECT_RE);
    if (genericMatch) {
      return {
        type: genericMatch[1].toLowerCase(),
        scope: genericMatch[2] ? genericMatch[2].trim() : null,
        breaking: true,
        description: genericMatch[3].trim(),
      };
    }
  }

  return null;
}

/**
 * Parse git commit objects ({ fullSha, shortSha, subject, body }) into structured entries.
 * Supports multi-entry commit bodies and BREAKING CHANGE footers.
 */
export function parseCommitEntries(commits) {
  const entries = [];

  for (const commit of commits) {
    const fullSha = commit.fullSha ?? "";
    const shortSha = commit.shortSha ?? (fullSha ? fullSha.slice(0, 7) : "");
    const rawSubjectLines = (commit.subject ?? "").split(/\r?\n/);
    const subject = (rawSubjectLines[0] ?? "").trim();
    const extraSubjectLines = rawSubjectLines.slice(1);
    const body = [...extraSubjectLines, ...(commit.body ? commit.body.split(/\r?\n/) : [])].join("\n");

    if (!subject && !body.trim()) {
      continue;
    }

    const bodyLines = body
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean);

    const breakingFooters = [];
    const extraConventional = [];

    for (let i = 0; i < bodyLines.length; i += 1) {
      const line = bodyLines[i];

      const footerMatch = line.match(BREAKING_FOOTER_RE);
      if (footerMatch) {
        let message = footerMatch[1].trim();
        if (!message && i + 1 < bodyLines.length) {
          const nextLine = bodyLines[i + 1];
          if (!BREAKING_FOOTER_RE.test(nextLine) && !BODY_CONVENTIONAL_RE.test(nextLine)) {
            message = nextLine;
            i += 1;
          }
        }
        if (message) {
          breakingFooters.push(message);
        }
        continue;
      }

      const bodyMatch = line.match(BODY_CONVENTIONAL_RE);
      if (bodyMatch) {
        const parsed = {
          type: bodyMatch[1].toLowerCase(),
          scope: bodyMatch[2] ? bodyMatch[2].trim() : null,
          breaking: Boolean(bodyMatch[3]),
          description: bodyMatch[4].trim(),
        };
        if (!isReleaseBumpHeader(parsed.type, parsed.scope)) {
          extraConventional.push(parsed);
        }
      }
    }

    const hasBreakingFooter = breakingFooters.length > 0;

    if (subject) {
      const parsedSubject = parseConventionalLine(subject, true);
      if (parsedSubject) {
        if (!isReleaseBumpHeader(parsedSubject.type, parsedSubject.scope)) {
          entries.push({
            kind: "commit",
            fullSha,
            shortSha,
            type: parsedSubject.type,
            scope: parsedSubject.scope,
            breaking: parsedSubject.breaking,
            commitHasBreakingFooter: hasBreakingFooter,
            description: parsedSubject.description,
          });
        }
      } else {
        entries.push({
          kind: "commit",
          fullSha,
          shortSha,
          type: null,
          scope: null,
          breaking: false,
          commitHasBreakingFooter: hasBreakingFooter,
          description: subject,
        });
      }
    }

    for (const extra of extraConventional) {
      entries.push({
        kind: "commit",
        fullSha,
        shortSha,
        type: extra.type,
        scope: extra.scope,
        breaking: extra.breaking,
        commitHasBreakingFooter: hasBreakingFooter,
        description: extra.description,
      });
    }

    for (const footerText of breakingFooters) {
      entries.push({
        kind: "breaking-footer",
        fullSha,
        shortSha,
        type: "breaking",
        scope: null,
        breaking: true,
        commitHasBreakingFooter: true,
        description: footerText,
      });
    }
  }

  return entries;
}

/**
 * Route parsed commit entries into breaking, features, design, and tech buckets.
 */
export function categorizeEntries(entries) {
  const categorized = {
    breaking: [],
    features: [],
    design: [],
    tech: [],
  };

  for (const entry of entries) {
    const type = entry.type ? entry.type.toLowerCase() : null;
    const scopeLower = entry.scope ? entry.scope.toLowerCase() : null;

    if (
      entry.kind === "breaking-footer" ||
      type === "breaking" ||
      (entry.breaking && !entry.commitHasBreakingFooter)
    ) {
      categorized.breaking.push(entry);
      continue;
    }

    if ((type && DESIGN_TYPES.has(type)) || (scopeLower && DESIGN_SCOPES.has(scopeLower))) {
      categorized.design.push(entry);
      continue;
    }

    if (type && FEATURE_TYPES.has(type)) {
      categorized.features.push(entry);
      continue;
    }

    categorized.tech.push(entry);
  }

  return categorized;
}

function capitalizeFirst(text) {
  if (!text) {
    return "";
  }
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function formatEntryLine(entry, sectionKey, repoUrl) {
  const description = capitalizeFirst(entry.description.trim());
  const redundantScopes = REDUNDANT_SCOPES_BY_SECTION[sectionKey];
  const scopeLower = entry.scope ? entry.scope.toLowerCase() : null;
  const includeScope = Boolean(entry.scope && (!redundantScopes || !redundantScopes.has(scopeLower)));

  const body = includeScope ? `**${entry.scope}** : ${description}` : description;

  let shaSuffix = "";
  if (entry.shortSha) {
    shaSuffix =
      repoUrl && entry.fullSha
        ? ` ([\`${entry.shortSha}\`](${repoUrl}/commit/${entry.fullSha}))`
        : ` (\`${entry.shortSha}\`)`;
  }

  return `- ${body}${shaSuffix}`;
}

function renderSectionLines(entries, sectionKey, repoUrl) {
  const seen = new Set();
  const lines = [];

  for (const entry of entries) {
    const line = formatEntryLine(entry, sectionKey, repoUrl);
    if (!seen.has(line)) {
      seen.add(line);
      lines.push(line);
    }
  }

  return lines;
}

function renderDownloadsSection(assets) {
  const lines = ["### 📥 Téléchargements", ""];

  if (!assets || assets.length === 0) {
    lines.push("- **macOS (Universal — Apple Silicon & Intel)** : installateur `.dmg`");
    lines.push("- **Windows (x64)** : installateurs `.exe` / `.msi`");
    return lines;
  }

  const macAssets = [];
  const winAssets = [];
  const otherAssets = [];

  for (const asset of [...assets].sort()) {
    const lower = asset.toLowerCase();
    if (lower.endsWith(".dmg")) {
      macAssets.push(`\`${asset}\``);
    } else if (lower.endsWith(".exe") || lower.endsWith(".msi")) {
      winAssets.push(`\`${asset}\``);
    } else {
      otherAssets.push(`\`${asset}\``);
    }
  }

  if (macAssets.length > 0) {
    lines.push(`- **macOS (Universal — Apple Silicon & Intel)** : ${macAssets.join(", ")}`);
  }
  if (winAssets.length > 0) {
    lines.push(`- **Windows (x64)** : ${winAssets.join(", ")}`);
  }
  if (otherAssets.length > 0) {
    lines.push(`- **Autres packages** : ${otherAssets.join(", ")}`);
  }

  return lines;
}

/**
 * Render formatted French markdown release notes from categorized entries.
 */
export function renderReleaseNotes({
  tag,
  previousTag = null,
  repoUrl = null,
  categorized,
  assets = [],
}) {
  const lines = [`## 🚀 >AIx Studio ${tag}`, ""];

  const sectionSpecs = [
    { key: "breaking", heading: "### 💥 Breaking changes" },
    { key: "features", heading: "### 📦 Fonctionnalités" },
    { key: "design", heading: "### 🎨 Design" },
    { key: "tech", heading: "### 🛠️ Tech" },
  ];

  let renderedAnySection = false;
  for (const spec of sectionSpecs) {
    const items = categorized?.[spec.key] ?? [];
    if (items.length === 0) {
      continue;
    }
    const formatted = renderSectionLines(items, spec.key, repoUrl);
    if (formatted.length === 0) {
      continue;
    }
    renderedAnySection = true;
    lines.push(spec.heading, "", ...formatted, "");
  }

  if (!renderedAnySection) {
    lines.push(
      "### 🛠️ Tech",
      "",
      `- Maintenance et publication de la version \`${tag}\`.`,
      "",
    );
  }

  lines.push("---", "");
  lines.push(...renderDownloadsSection(assets));

  if (previousTag && repoUrl) {
    lines.push(
      "",
      `**Changelog complet** : [\`${previousTag}...${tag}\`](${repoUrl}/compare/${previousTag}...${tag})`,
    );
  }

  return `${lines.join("\n").trimEnd()}\n`;
}

function readCommitsInRange(previousTag, currentTag) {
  const range = previousTag ? `${previousTag}..${currentTag}` : currentTag;
  const raw = runGit(
    ["log", "--no-merges", "--format=%H%x1f%h%x1f%B%x1e", range],
    { allowFailure: false },
  );

  if (!raw) {
    return [];
  }

  return raw
    .split("\x1e")
    .map((record) => record.replace(/^\r?\n+/, ""))
    .filter(Boolean)
    .map((record) => {
      const [fullSha = "", shortSha = "", ...messageParts] = record.split("\x1f");
      const rawMessage = messageParts.join("\x1f").trimEnd();
      const [firstLine = "", ...restLines] = rawMessage.split(/\r?\n/);
      return {
        fullSha: fullSha.trim(),
        shortSha: shortSha.trim(),
        subject: firstLine.trim(),
        body: restLines.join("\n"),
      };
    });
}

function listAssetsInDir(assetsDir) {
  if (!assetsDir) {
    return [];
  }
  const resolved = path.resolve(assetsDir);
  if (!fs.existsSync(resolved)) {
    return [];
  }
  return fs
    .readdirSync(resolved, { withFileTypes: true })
    .filter((entry) => entry.isFile() && !entry.name.startsWith("."))
    .map((entry) => entry.name)
    .sort();
}

function resolveDefaultTag() {
  if (process.env.GITHUB_REF_NAME) {
    return process.env.GITHUB_REF_NAME;
  }
  const exactTag = runGit(["describe", "--tags", "--exact-match", "HEAD"], { allowFailure: true });
  return exactTag || "HEAD";
}

function parseCliArgs(argv) {
  const opts = {
    tag: null,
    from: null,
    assetsDir: null,
    output: null,
    repoUrl: null,
  };

  const readValue = (index, flag) => {
    const val = argv[index + 1];
    if (!val) {
      throw new Error(`Option ${flag} requires a value.`);
    }
    return val;
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--tag") {
      opts.tag = readValue(i, arg);
      i += 1;
    } else if (arg.startsWith("--tag=")) {
      opts.tag = arg.slice("--tag=".length);
    } else if (arg === "--from") {
      opts.from = readValue(i, arg);
      i += 1;
    } else if (arg.startsWith("--from=")) {
      opts.from = arg.slice("--from=".length);
    } else if (arg === "--assets-dir") {
      opts.assetsDir = readValue(i, arg);
      i += 1;
    } else if (arg.startsWith("--assets-dir=")) {
      opts.assetsDir = arg.slice("--assets-dir=".length);
    } else if (arg === "--output" || arg === "-o") {
      opts.output = readValue(i, arg);
      i += 1;
    } else if (arg.startsWith("--output=")) {
      opts.output = arg.slice("--output=".length);
    } else if (arg === "--repo-url") {
      opts.repoUrl = readValue(i, arg);
      i += 1;
    } else if (arg.startsWith("--repo-url=")) {
      opts.repoUrl = arg.slice("--repo-url=".length);
    } else {
      throw new Error(`Unknown option: ${arg}`);
    }
  }

  return opts;
}

function main() {
  const opts = parseCliArgs(process.argv.slice(2));
  const tag = opts.tag || resolveDefaultTag();
  const previousTag = opts.from || findPreviousTag(tag);
  const repoUrl = opts.repoUrl ? opts.repoUrl.replace(/\/+$/, "") : inferRepoUrl();
  const commits = readCommitsInRange(previousTag, tag);
  const entries = parseCommitEntries(commits);
  const categorized = categorizeEntries(entries);
  const assets = listAssetsInDir(opts.assetsDir);

  const markdown = renderReleaseNotes({
    tag,
    previousTag,
    repoUrl,
    categorized,
    assets,
  });

  if (opts.output) {
    const outPath = path.resolve(opts.output);
    fs.mkdirSync(path.dirname(outPath), { recursive: true });
    fs.writeFileSync(outPath, markdown, "utf8");
  } else {
    process.stdout.write(markdown);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main();
}
