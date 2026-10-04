import { paths } from "../lib/paths.mjs";
import { generateKeyPairSync } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { read, root, write } from "../lib/common.mjs";

const [keyId, privateFile, clientTrustFile] = process.argv.slice(2);
if (
  !clientTrustFile ||
  !/^[a-z0-9][a-z0-9._-]{0,127}$/.test(keyId) ||
  keyId.includes("..")
)
  throw new Error(
    "Usage: init-signing-key.mjs <key-id> <private-file-outside-repos> <client-trust.rs>",
  );
let output = path.resolve(privateFile);
const client = path.resolve(clientTrustFile);
const platformRoot = path.resolve(path.dirname(client), "../../../..");
function outsideRepositories(file) {
  for (const repository of [realpathSync(root), realpathSync(platformRoot)]) {
    const relative = path.relative(repository, file);
    if (
      !relative ||
      (!relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative))
    )
      throw new Error("Private key must be outside both repositories");
  }
}
outsideRepositories(output);
if (existsSync(output)) throw new Error("Refusing to overwrite a private key");
const keys = read(paths.keys);
if (keys.keys.some((key) => key.id === keyId))
  throw new Error("Signing key ID already exists");
const source = readFileSync(client, "utf8");
const marker =
  /pub const OFFICIAL_PLUGIN_KEYS: &\[\(&str, &str\)\] = &\[([\s\S]*?)\];/;
if (!marker.test(source))
  throw new Error("Client trust store declaration not found");
const { publicKey, privateKey } = generateKeyPairSync("ed25519");
const raw = publicKey
  .export({ type: "spki", format: "der" })
  .subarray(-32)
  .toString("base64");
mkdirSync(path.dirname(output), { recursive: true });
output = path.join(realpathSync(path.dirname(output)), path.basename(output));
outsideRepositories(output);
writeFileSync(output, privateKey.export({ type: "pkcs8", format: "pem" }), {
  flag: "wx",
  mode: 0o600,
});
if (process.platform === "win32") {
  const identity = spawnSync("whoami", [], {
    encoding: "utf8",
    windowsHide: true,
  });
  const acl =
    identity.status === 0
      ? spawnSync(
          "icacls",
          [
            output,
            "/inheritance:r",
            "/grant:r",
            `${identity.stdout.trim()}:(F)`,
          ],
          { encoding: "utf8", windowsHide: true },
        )
      : null;
  if (acl?.status !== 0) {
    unlinkSync(output);
    throw new Error(
      "Unable to restrict private key ACL to its owner; no key was retained",
    );
  }
}
keys.keys.push({
  id: keyId,
  algorithm: "ed25519",
  publicKey: raw,
  status: "active",
  purpose: "repository-package-signing",
});
write(paths.keys, keys);
writeFileSync(
  client,
  source.replace(
    marker,
    (_, existing) =>
      `pub const OFFICIAL_PLUGIN_KEYS: &[(&str, &str)] = &[${existing}\n    ("${keyId}", "${raw}"),\n];`,
  ),
);
process.stdout.write(
  `Public key pinned in Store metadata and client. Private key saved outside Git: ${output}\nConfigure plugin-signing environment reviewers before uploading NYATERM_STORE_SIGNING_KEY.\n`,
);
