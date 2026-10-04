import { paths } from "../lib/paths.mjs";
import { buildCatalog, validateRecords } from "../lib/records.mjs";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { unzipSync } from "fflate";
import {
  download,
  inspect,
  matchManifest,
  read,
  schema,
  verifySignature,
} from "../lib/common.mjs";

const { plugins, keys } = validateRecords();
const expected = buildCatalog(plugins);
const actual = read(paths.catalog);
schema("catalog", actual);
if (JSON.stringify(expected) !== JSON.stringify(actual))
  throw new Error("Catalog is stale; run npm run catalog");
if (process.argv.includes("--packages")) {
  const work = mkdtempSync(path.join(os.tmpdir(), "nyaterm-store-"));
  try {
    for (const plugin of plugins) {
      for (const release of [
        ...plugin.versions.flatMap((v) =>
          v.artifacts.map((a) => ({
            ...a,
            version: v.version,
            permissions: v.permissions,
            signed: true,
          })),
        ),
        ...(plugin.candidates ?? []),
      ]) {
        const bytes = await download(release);
        const file = path.join(work, "candidate.nyap");
        writeFileSync(file, bytes);
        // Validate and bound the ZIP in Rust before any JS decompression.
        const preview = inspect(file);
        matchManifest(preview.manifest, plugin, release);
        if (release.signed) {
          const entries = unzipSync(bytes);
          if (!entries["signature.json"])
            throw new Error("Missing repository signature");
          const signature = JSON.parse(
            Buffer.from(entries["signature.json"]).toString("utf8"),
          );
          verifySignature(
            entries["checksums.json"],
            signature,
            keys.get(release.signingKeyId),
          );
        } else if (preview.signature.status !== "unsigned")
          throw new Error("Candidates must be unsigned");
      }
    }
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}
process.stdout.write(
  `Validated ${plugins.length} plugin records and generated catalog.\n`,
);
