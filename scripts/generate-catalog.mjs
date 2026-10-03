import { buildCatalog, validateRecords, write } from "./common.mjs";
write("catalog/index.json", buildCatalog(validateRecords().plugins));
