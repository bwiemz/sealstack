# engineering-context example

End-to-end walkthrough: apply a schema, ingest local markdown, query it.

```bash
signet dev
signet schema apply schemas/doc.csl
signet connector add local-files --root ./sample-docs
signet connector sync local-files
signet query "what does the setup guide say about postgres?"
```

The query should return hits with scores > 0 and a receipt ID. Fetch the
receipt at `http://localhost:7070/v1/receipts/:id` to see the source files
that contributed.
