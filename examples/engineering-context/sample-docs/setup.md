# Setup guide

## Postgres

SealStack stores typed rows and receipt metadata in PostgreSQL 16.
The local `sealstack dev` stack brings up Postgres on port 5432 with database,
user, and password all set to `cfg`.

## Qdrant

Vector embeddings live in Qdrant. The dev stack exposes the HTTP API on
6333 and the gRPC API on 6334.

## Redis

Redis is used for the ingestion job queue.
