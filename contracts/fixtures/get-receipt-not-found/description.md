# get-receipt-not-found

`GET /v1/receipts/<unknown-id>` returns HTTP 404 with the `not_found`
error code; SDK maps this to `NotFoundError`. Tests the SDK's mapping
for the most common 4xx outside of authz, and is the receipts namespace
counterpart to `get-schema-not-found`.
