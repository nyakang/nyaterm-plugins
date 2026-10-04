import { fileURLToPath } from "node:url";

export const root = fileURLToPath(new URL("../../", import.meta.url));
export const paths = Object.freeze({
  extensions: "extensions",
  publishers: "trust/publishers",
  keys: "trust/signing-keys.json",
  revoked: "trust/revoked.json",
  schemas: "tooling/schemas",
  catalog: "public/v1/plugins.json",
});

export const extensionPaths = (id) => ({
  listing: `${paths.extensions}/${id}/listing.json`,
  releases: `${paths.extensions}/${id}/releases`,
  submissions: `${paths.extensions}/${id}/submissions`,
});
