# SealStack SDK ↔ Gateway compatibility

| SDK version | Gateway version | Status |
|-------------|-----------------|--------|
| 0.3.x       | 0.3.x           | supported |
| 0.3.x       | 0.2.x and earlier | untested |

## Skew policy

**SDK X.Y supports gateway X.Y and gateway X.(Y-1).** Lets operators
choose deploy order (SDK-first or gateway-first) for rolling updates.
Older-than-(Y-1) is explicitly out of scope.

This matrix becomes load-bearing post-1.0; pre-1.0 it documents the
single supported pair plus the policy that will govern future rows.
