# Delivery ordering with `create_scheduler`

A small workspace whose [`.aspect/config.axl`](.aspect/config.axl) sets
`DeliveryTrait.create_scheduler` to deliver targets in dependency order:

```
migrate ──► api ──► web
worker                     (no prerequisites)
```

Each deliverable lists its prerequisites as `deliver_after=<label>` tags in
[`BUILD.bazel`](BUILD.bazel). The factory reads them with one `bazel query`
before the release build, and the selector starts a built target once every
prerequisite has finished as `ok` or `skip`.

## Run it

From this directory:

```bash
aspect delivery --mode=always --track-state=false --max-parallelization=4
```

`--mode=always --track-state=false` delivers every target without a state
backend, so it runs anywhere. `migrate` and `worker` start together; `api`
starts when `migrate` finishes, and `web` when `api` does:

```
  Delivering //:migrate (FORCED)...
  Delivering //:worker (FORCED)...
21:17:18 worker: delivering
21:17:19 worker: done
21:17:18 migrate: delivering
21:17:21 migrate: done
  Delivering //:api (FORCED)...
21:17:21 api: delivering
21:17:23 api: done
  Delivering //:web (FORCED)...
21:17:23 web: delivering
21:17:24 web: done
```

`./test.sh <path to aspect-cli>` runs the same command and checks that order;
CI runs it on every change.

To see a blocked target, make `migrate` fail (change its delay argument to
`x` in `BUILD.bazel`): `api` and `web` then fail as `scheduler_blocked` while
`worker` is still delivered.
