# Docker pin-of-record

`NOCKCHAIN_COMMIT` holds the pinned nockchain monorepo SHA that local
`Dockerfile`s must pass as the `NOCKCHAIN_COMMIT` build-arg. The
Dockerfile itself is gitignored (project policy: local dev only, not
shipped), so this file is the authoritative record of what version of
upstream nockchain a container build should use.

## AUDIT 2026-04-17 M-08

Before this pin existed, Dockerfiles used `git clone --depth 1
--branch main`. Every image rebuild silently picked up whatever
upstream `main` said that day — jet regressions, tx-engine breaks, or
hoonc compiler drift all landed without review. The pin fixes that.

## Bumping the pin

1. Pick a commit on `zorp-corp/nockchain` `main` that you want to
   adopt.
2. Update this file.
3. Update the matching comment in `hull/Cargo.toml` and
   `hull-rag/Cargo.toml` ("Nockchain monorepo pin: ...").
4. Rebuild the local container with
   `docker build --build-arg NOCKCHAIN_COMMIT=$(cat docker/NOCKCHAIN_COMMIT) ...`
5. Run the test matrix before merging.

## Dockerfile snippet

```dockerfile
ARG NOCKCHAIN_COMMIT=505c3ea586bacfece2d451fbd01dfa18105facea
RUN git clone https://github.com/zorp-corp/nockchain.git $WORKSPACE/nockchain \
    && git -C $WORKSPACE/nockchain checkout ${NOCKCHAIN_COMMIT}
```

Keep the default `ARG` value in sync with this file.
