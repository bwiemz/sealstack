# apply-ddl-success

`POST /v1/schemas/examples.Doc/ddl` applies the Postgres DDL emitted
by the CSL codegen for the named schema. Response surfaces the count
of statements actually applied so the SDK can report partial-progress
on multi-statement bodies.
