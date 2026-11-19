#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const SEMVER_REGEX = /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/;
const DEFAULT_CONF_PATH = "src-tauri/tauri.conf.json";

/**
 * Parse CLI arguments for sync-version.
 * Supports: node sync-version.mjs <tag> [--file <path>]
 */
function parseArgs(argv) {
  let tag = null;
  let filePath = DEFAULT_CONF_PATH;

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--file" || arg === "-f") {
      const next = argv[i + 1];
      if (!next) {
        throw new Error(`Option ${arg} requires a file path.`);
      }
      filePath = next;
      i += 1;
    } else if (arg.startsWith("--file=")) {
      filePath = arg.slice("--file=".length);
      if (!filePath) {
        throw new Error("Option --file= requires a non-empty file path.");
      }
    } else if (!arg.startsWith("-") && tag === null) {
      tag = arg;
    } else {
      throw new Error(`Unexpected argument: ${arg}`);
    }
  }

  return { tag, filePath };
}

function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    console.error(`[sync-version] ${err.message}`);
    console.error("Usage: node .github/scripts/sync-version.mjs <tag> [--file <path>]");
    process.exit(1);
  }

  if (!args.tag) {
    console.error("[sync-version] Missing <tag> argument.");
    console.error("Usage: node .github/scripts/sync-version.mjs <tag> [--file <path>]");
    process.exit(1);
  }

  const version = args.tag.replace(/^v/, "");
  if (!SEMVER_REGEX.test(version)) {
    console.log(
      `[sync-version] Tag "${args.tag}" (parsed "${version}") is not a valid semver string. Skipping version sync without modifying ${args.filePath}.`,
    );
    process.exit(0);
  }

  const resolvedPath = path.resolve(args.filePath);
  const raw = fs.readFileSync(resolvedPath, "utf8");
  const conf = JSON.parse(raw);

  const previousVersion = conf.version;
  conf.version = version;

  fs.writeFileSync(resolvedPath, `${JSON.stringify(conf, null, 2)}\n`, "utf8");
  console.log(
    `[sync-version] Synced ${args.filePath} version: ${previousVersion ?? "(unset)"} -> ${version} (from tag ${args.tag}).`,
  );
}

main();
