# engineering-context example

End-to-end walkthrough: apply a schema, ingest local markdown, query it.

```bash
cfg dev
cfg schema apply schemas/doc.csl
cfg connector add local-files --root ./sample-docs
cfg connector sync local-files
cfg query "what does the setup guide say about postgres?"
```

The query should return hits with scores > 0 and a receipt ID. Fetch the
receipt at `http://localhost:7070/v1/receipts/:id` to see the source files
that contributed.
