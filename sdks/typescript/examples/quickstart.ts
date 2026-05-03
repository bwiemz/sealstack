import { SealStack } from "@sealstack/client";

const client = SealStack.bearer({ url: "http://localhost:7070", token: "dev-token" });
await client.admin.schemas.register({ meta: { /* compiled schema */ } });
await client.admin.schemas.applyDdl("examples.Doc", { ddl: "/* ddl */" });
await client.admin.connectors.register({ kind: "local-files", schema: "examples.Doc", config: { root: "./docs" } });
await client.admin.connectors.sync("local-files/examples.Doc");
const result = await client.query({ schema: "examples.Doc", query: "getting started" });
console.log(result);
