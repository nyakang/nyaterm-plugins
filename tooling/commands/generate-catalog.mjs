import { write } from "../lib/common.mjs";
import { buildCatalog, validateRecords } from "../lib/records.mjs";
import { paths } from "../lib/paths.mjs";
write(paths.catalog, buildCatalog(validateRecords().plugins));
