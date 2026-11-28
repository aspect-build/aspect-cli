# Dashboard counts

What the Dashboard's Actions and Tests rows count, and how the failure summary
picks its entries: every input, rule, timing and category, as the code has
them. The failure summary is the same with the Dashboard or without it.

AXL paths are under `crates/aspect-cli/src/builtins/aspect/`.

| File | What it does |
|---|---|
| `private/lib/failure_records.axl` | Takes in each build event (`collect_event`): the counts (`count_event`), the failure records (`note_event`) and the root causes (`note_blocked`); and each execution log spawn (`count_spawn`) |
| `feature/dashboard/actions.axl` | The bar, the Actions and Tests rows, Metrics' actions: `action_split`, `action_parts`, `action_results`, `remainder_words`, `test_progress` |
| `private/lib/failures.axl` | The failure summary (`failure_summary`) and its entries (`failure_entries`, `entry_text`) |
| `private/lib/failure_ties.axl` | What of Bazel's console ties to an entry (`console_ties`) |
| `feature/dashboard.axl` | The hooks that feed them: `build_event`, `exec_log_event`, `output_frame`, `output_end` |
| `crates/bazel/src/console/report.rs` | `ErrorCollector`, which keeps Bazel's `ERROR:` blocks for the ties |

## Inputs

| Source | What is read | When it arrives |
|---|---|---|
| `ActionExecuted`, failed | primary output, mnemonic (`type`), `failure_detail` (message, category, code, `spawn_exit_code`), stderr and stdout | live; Bazel publishes every failed action |
| `ActionExecuted`, succeeded | one more action seen (`actions_seen`); internal when it has no `command_line`; its stdout and stderr, when it has them | live, only with `--build_event_publish_all_actions` |
| `TargetConfigured` | configured targets, test targets (`target_kind` ending `_test rule`); Aborted: the target failed or was skipped, and its root causes (`children`) | live |
| `TargetComplete` | built or failed; root causes (`children`); a failed one with no failed action among its children: its `failure_detail` message | live |
| `Aborted` | the `reason` and `description` under `configured_label`, `unconfigured_label`, `pattern_expanded`, `target_configured`, `target_completed` and `test_summary` ids | live |
| `TestResult` | status, attempt, run and shard, `cached_locally`, `execution_info.cached_remotely`, duration, `test.log` | live, one per attempt |
| `TestSummary` | `overall_status`, `run_count`, `shard_count` | live, after a test's last attempt |
| `BuildStarted` | the command (`test` or `coverage` tests), the workspace directory | first |
| `OptionsParsed` | `cmd_line`: the cache flags and an execution log of the user's own | at the start |
| `WorkspaceConfig`, `Configuration` | the exec root and each output directory, to find a test's log | at the start |
| `progress` | its stderr's last `[n / m]` (`bep_progress`) | live; the last one at the end |
| `BuildFinished` | the exit code's name | at the end |
| `BuildMetrics` | `action_summary`: `actions_executed`, `runner_count`, `action_cache_statistics.hits`, `actions_created`; for Metrics also the misses, their reasons and `action_data` | at the end, after `BuildFinished` |
| The console's progress area (`build.output_feed()`) | the `[n / m]` status line, the `Analyzing:` line, the running actions, `3 / 9 tests` | live, every frame (about 100 ms) |
| The compact execution log (`ExecLogHook`) | each spawn's runner, `cache_hit` and first output; the path of each file under `/testlogs/` | live, as entries come, drained to its end before the build ends |
| The collected errors (a `bazel.output.collect_errors()` stage, `build.output_collected_errors()`) | the `ErrorCollector`'s blocks (`error`, `from`, `tail`), each `ERROR:` header's `location`, `message`, `coloured`, `boilerplate` and `body`; `not_kept`, `froms_not_kept`, `stdout`, `color`, `relay_cap` | once the console has ended |
| Logs on disk | a failed action's stderr and stdout (the task history's copy first, `kept_action_log`), a test's `test.log`, Bazel's raw console log (`build.output_log`) | read when an excerpt or a tie needs them |

A `bytestream://` log is looked for under the runner's bb_clientd mount, and a
test's first in the output tree, at the path its summary gives
(`local_test_logs`). `BuildMetrics`' `build_graph_metrics` is not read (see
[Known limits](#known-limits)).

### Flags that change what arrives

| Flag or setting | Effect |
|---|---|
| `--dashboard:actions=all` (the default) | The CLI adds `--build_event_publish_all_actions` for the task alone (`publish_all_actions_unless_sent_out`) |
| No Dashboard | The CLI adds no `--build_event_publish_all_actions` (but where `ArtifactUpload` uploads with `upload_build_logs = "all"`), so the failure summary has no successful action's output to read (`_absorb_tails`) |
| `--dashboard:actions=failed` | Only failed actions' events: while the build runs, what the execution log has not told apart is `? unreported` |
| `--bes_backend`, or `--remote_cache` or `--remote_executor` without `--experimental_build_event_upload_strategy=local` | Bazel sends the events out itself (`bazel_sends_events_out`), so the CLI does not add the flag: as `failed` |
| Your own `--build_event_publish_all_actions` | Every action's event, whatever the above say |
| BES backends the CLI streams to, `--build_event_binary_file` | No change: the runtime keeps the extra events from them unless your own flags ask for every action |
| `--execution_log_binary_file`, `--execution_log_json_file` | Bazel writes no compact log for the Dashboard (`own_execution_log`; the last value counts, and an empty or false one or the `--no` form clears it): while the build runs only `━ N done`; at the end `runner_count` decides the kinds |
| `--disk_cache`, `--remote_cache`, `--remote_executor` | Which cache parts show at the end with no hits, dimmed (`cache_kinds`: the last value given, when not empty) |
| `--nokeep_going` | Bazel stops at the first failed action or target: `[n / m]` ends short of its total (`─` on the bar), and the tests it did not get to are `skipped` |
| `--experimental_ui_max_stdouterr_bytes` | The relay cap the action ties expect (`relay_cap`; 0 and -1 included) |
| `--color=no` | The collector takes an uncoloured `ERROR:` as Bazel's, and output is compared without escapes, as Bazel relays it then |
| `--show_timestamps` | The `(hh:mm:ss) ` prefix is left out before a line is read (`bazel_line`, and the collector's own) |
| `--ui_actions_shown` | Caps the tests counted running |

## The Actions row

The Results pane shows the newest build's actions. The bar above the panes
draws the same parts, and the Metrics tab lists them with what makes up each
(`action_lines`), for the build `<` and `>` pick in the Metrics and Invocation
tabs, the newest unless one was picked. Every part is in one unit, Bazel's
actions.

```
Actions  ran 8: █ 5 ok · ╳ 3 failed
         cache hits 4:
           ▒ 3 local · ▓ 1 disk
         ○ 49 skipped · ░ 1 internal
```

### The total and the hit rate

Once the build has ended with its `BuildMetrics`, the bar's label (and the
build's folded row) says `N actions · H cache hits (P%)` (`actions_text`):

- **N** is the done count of Bazel's last `[n / m]` (`build_progress`): the
  larger of the last count a `progress` event carried and the last a frame read
  from the console, since either can miss Bazel's final redraw. For a build
  Bazel never counted (a BUILD file that does not parse), N is the parts' sum.
- **H** is `local + disk + remote` (`hit_count`).
- **P** is H / N to the nearest whole percent, but 1 for any hit and 99 for all
  but some, so only none says 0% and only all says 100% (`hit_percent`).
  Everything else in N counts as a miss, `internal` and the `○` actions too.

While the build runs the label is `3,830 of 8,563 actions`, with `+` after the
total while Bazel is still analysing and may find more; with the bar not drawn,
the header says `3,830 of 8,563 actions done` (`phase_status`). A build that
ended without `BuildMetrics` has no hit rate: its bar says
`3,830 of 8,563 actions done`, its folded row `3,830 actions`.

### The parts

The live column is for a build with every action's events; the next section
says what changes without them.

| Part | Glyph | At the end (`final`) | While the build runs |
|---|---|---|---|
| `ok` | `█` | Executed actions that succeeded: the execution log's actions any spawn of which ran (`log_kinds`); without the log, the spawns of every `runner_count` runner but the caches, `internal` and `total`, capped so that `ok`, `disk` and `remote` together never pass the executed actions that are not internal | Successful actions with a command line, but those the log says a cache served |
| `failed` | `╳` | Every action the events say failed (`action_failures`), a test's own `TestRunner` too | The same |
| `local` | `▒` | `action_cache_statistics.hits` (the `action cache hit` runner when there are no statistics) | `~N`, an estimate: Bazel's done count past every event, once that gap has aged |
| `disk` | `▓` | The log's actions a disk cache served every spawn of; without the log, the `disk cache hit` runner | The log's |
| `remote` | `▓` | The same for a remote cache (a remote executor's cached result too); without the log, every other `… cache hit` runner | The log's |
| `up to date` or `skipped` | `○` | N less every other part | Not shown |
| `internal` | `░` | `runner_count`'s `internal`, less the failed actions Bazel counted there | Successful actions with no command line |
| `other` | `▌` | Without the log: executed actions with a spawn that no runner accounts for. Rare | Not shown |
| `unreported` | `?` (`┈` on the bar) | Not shown | Done, but no event or log entry has said how yet |
| `running`, `pending` | `─` | Not shown | What runs now (`running_count`, at most what is not done) and the rest of the total |
| `done` | `━` | Not shown | The whole done count, with an execution log of your own |

Each failed action counts once, as `failed`:

- **`internal`** gives up the failed actions Bazel counted there: with every
  action's events, as many as it has beyond the successful internal actions
  seen; without them, one for each failure but those whose `failure_detail` code
  is `ACTION_OUTPUTS_NOT_CREATED` or `ACTION_FINALIZATION_FAILURE`, which Bazel
  counts under the runner its spawn ran in (`counted_internal`).
- **With the execution log,** its kinds give up each failed action found in it
  by primary output (`_logged_kinds`; a test's by the `test.xml` beside its
  `test.log`). Without it, the failures not taken from `internal` come out of
  `ok`, then `disk`, then `remote`.

The parts come in two groups, each headed by its total, then the rest:
`ran N: █ ok · ╳ failed`, `cache hits N: ▒ local · ▓ disk · remote`, then `○`,
`░`, `▌`, `?` and `─ N running · M pending`. A group with one part drops its
header and the part says the group's meaning (`█ 5,697 ran`,
`▒ 2,869 local action cache hits`). The local action cache's part says
`▒ N local action cache`, or `local` under 100 columns or where the whole words
don't fit. At the end `local` is kept even at 0, where there is room. `disk`
and `remote` are shown when they have hits, or at the end, dimmed at 0, when
the build was given that cache; `▓` is drawn once, before the first of them.
Where the rows have no room, the caches with no hits, the parts that count
nothing and `internal` go first, then parts go in `_DROP_ORDER` (`other`,
`pending`, the hits folded to `▒ N cache hits`, then the hits, `running`,
`unreported`, `ok`, `skipped`); `failed` never goes, and no part is cut but one
wider than a whole row.

Parts that add up to more than N are shown as counted, never cut to fit, and
`○` is then 0. [Known limits](#known-limits) says when that happens.

### The `○` actions

`○` is what Bazel's `[n / m]` counts done that it neither ran nor checked
against a cache: actions up to date in its memory, which
`ExecutionProgressReceiver` counts done as it evaluates them and no event or
metric counts, and actions an identical action of another target made. It is N
less every other part, so it has a count only from `BuildMetrics`, at the end.

Its words come from the build's outcome (`remainder_words`):

| Outcome | Words | Why |
|---|---|---|
| `BuildFinished` says `SUCCESS`, `TESTS_FAILED` or `NO_TESTS_FOUND` | `up to date` | Nothing but tests failed, and a failed test holds back no other action |
| `BuildFinished` says any other code | `skipped` | Actions whose input failed can be among them, and nothing Bazel publishes tells them apart |
| `BuildMetrics` came but no `BuildFinished` | `skipped` once a target failed (its build, analysis or loading) or an action other than a `TestRunner` failed, else `up to date` | The failures seen as the build ran |

`remainder_words` reads the third rule while the build runs too (a failure seen
stays seen, so its words change at most once), but no part draws `○` before the
end. The `?` overlay gives both words' lines (`_REMAINDER_LEGEND`) and the
Bazel 7 and 8 note (`_REMAINDER_NOTE`).

### Live, and at the end

While the build runs, every number is what has arrived so far:

- **The total** grows while Bazel analyses (`+`) and as it walks the graph.
- **With every action's events** (`_published_split`), each successful action
  with a command line is `ok` until the execution log says a cache served every
  spawn of it, so `ok` can drop as `disk` and `remote` rise. Events ahead of
  Bazel's count come out of `ok`.
- **The gap** between Bazel's done count and the events seen is the local action
  cache's (`~N local`, an estimate, after a failure too) only once it has
  outlasted the count it opened at (`settled`, set by `note_progress` each time
  the count moves): Bazel's count can lead the events by a frame. A younger gap
  is `? unreported`.
- **Without every action's events,** `ok`, `disk` and `remote` are the
  execution log's kinds, at most the done count; a failed action's spawn counts
  as `ok`, and the rest is `? unreported`. The failures come apart at the end.
- **With an execution log of your own,** only `━ N done`, and what runs and is
  pending.
- **`─ N running · M pending`:** running is the larger of the rows Bazel lists
  and the `N running` on its status line, at most what is not done; pending is
  the rest of the total.

A live part shown once (`unreported`, `running`, `pending`, `internal`) keeps
its place at 0 (`? 0 unreported`), and a part's width only grows, while the
pane grows no bigger (`_actions_memory`), so parts don't come and go as counts
move. At the end, `BuildMetrics` replaces all of it with the exact split above:
`~local` becomes the action cache's hits, and what was `?` falls into its kind
or into `○`.

### The bar

The bar draws a segment of each part, in the order `█`, `▓`, `▒`, `▌`, `░`, `╳`,
`○`, `┈`, then `─` for what is not done (`action_bar`), each in proportion to
its count and every part given at least a cell while one can be spared. A
failed build with no failed action and nothing left undone gets one `─` cell,
never a full bar. With an execution log of your own, the done count is one `━`
segment until the build ends.

## The Tests row

```
Tests    2 passed, 4 failed (2 cached) · ─ 1 running · 3 pending
```

| Count | From |
|---|---|
| passed | `TestSummary` with `overall_status` `passed` |
| failed | `TestSummary` with `failed`, `timeout`, `failed_to_build`, `remote_failure` or `tool_halted_before_testing`; a test target whose `TargetComplete` failed in a `test` or `coverage` build (Bazel posts no summary for it); an Aborted `TestSummary` of a test with a `FAILED_TO_BUILD` result, unless counted already |
| flaky | `TestSummary` with `flaky`: passed in the end |
| skipped | An Aborted `TestSummary` with no `FAILED_TO_BUILD` result (a build without `--keep_going` stopped first); an interrupt is no verdict |
| cached | Test targets with a summary every `TestResult` of which says `cached_locally` or `cached_remotely`, at most passed + failed + flaky (`tests_cached_text`) |

A count of 0 is left out (`0 passed` before any test finished), and the row is
hidden for a build with no test targets and no results. A narrow pane writes
`Tests   ✗4 ✓2 ~1 ⊘1 (2 cached)`.

While a `test` or `coverage` build runs, `─ N running · M pending` follows
(`test_progress`):

- **running:** test targets with a `Testing //t:a …` row in Bazel's running
  list, each once, without a summary or a failed build yet; at most
  `--ui_actions_shown` (8 by default), since Bazel lists no more.
- **pending:** the tests Bazel's status line says it runs (`3 / 9 tests`, at
  least those finished and running) less those finished (passed, failed, flaky,
  skipped) and running. Not shown while Bazel analyses, nor before its status
  line gives a test total.

## The failure summary

The Console feature prints it at the end of every build's console (`_summarize`,
an `output_end` hook ahead of the tips). Under the Dashboard it goes to the
scrollback when the screen is left (`report_lines`), never into the Console tab.
Either way `failure_summary` writes it from a state `collect_event` filled from
the build events.

It has a `── Failures (N) ──` section when anything failed, a
`── Other errors from Bazel ──` section when Bazel wrote errors no entry claims
(on a passing build too), then on every build the counts line and
`All of it: aspect history show <id>`.

### Entries

Entries come from build events alone, never from guessing at text:

| Event | Entry | Group |
|---|---|---|
| Aborted `unconfigured_label` | A root whose loading failed (`loading`), unless every target naming it as a root cause has an entry of its own, as each target of a missing source file does (`_target_entry`) | target |
| Aborted `configured_label` with reason `analysis_failure` or `loading_failure` | A root whose analysis failed (`analysis`) | target |
| Aborted `pattern_expanded` with a failing reason and a description | The pattern (`loading`) | target |
| Aborted `target_configured` or `target_completed` (not an aspect's) with a failing reason and a description | The target (`analysis`) | target |
| `TargetComplete` failed, with no failed action among its children and a `failure_detail` message | The target, its message quoted (`missing input file '//a:b'`) | target |
| `ActionExecuted` failed | The action | action |
| `TestResult` not `PASSED` (one of the failed statuses above) | The test: the last attempt of its first failed shard or run stands for it (`test_group`) | test |
| A failed target none of those is about and no root cause blocks | `Bazel's events give no reason; its error is in the console` (`_NO_REASON`) | target, after the tests |
| None of those, no other errors, and Bazel exited non-zero | `✗ Bazel exited N`, quoting the raw log's last `ERROR:` lines, else a crashed server's `FATAL:` lines, else its last lines but progress (`_last_words`) | target |

Each label has one target entry. A test attempt Bazel posts twice is one. A
failed `TestRunner` action (a test Bazel failed itself) is an action entry
standing for its run and shard, until a `TestResult` of them comes with a log,
which replaces it. A flaky test's failed attempts make no entry in the summary:
the Failures tab lists them, marked, and the Dashboard's scrollback lists them
under `Flaky (passed on a later attempt):`.

The groups come in order, targets, actions, tests, each in the order Bazel
posted them. Each target blocked by a failed dependency (`blocked_targets`: its
root causes, from the `children` of its failed `TargetComplete` or Aborted
`TargetConfigured`, are other labels, and it has no entry) goes under the first
entry of its first root cause, `○ blocked: //a, //b, //c +4 blocked`; a root
cause with no entry gets a line of its own.

### Headings and excerpts

Each entry has a heading, `✗ <label>  <kind> · <details>`:

- **an action:** its mnemonic, `exit N` from `failure_detail`'s
  `spawn_exit_code` (the event's own `exit_code` is 1 for every failed spawn),
  or the status Bazel's tied line gives (`killed`); for a command that timed out,
  ran out of memory or could not start in its sandbox, that code in words
  (`timeout`, `out of memory`, `subprocess start failed`); and its location,
  from a tie.
- **a test:** `test`, its shard and run where it has more than one, then
  `failed 2 of 2 attempts` and the time (from 0.1s), or
  `timed out after 5.0s`.
- **a target:** `analysis` or `loading` and its location; a missing source's
  has no kind.

The first five entries (`_EXCERPTED`) get an excerpt behind `│`: at most five
rows (`_EXCERPT_LINES`), each line at most 300 bytes and three rows, `│ …` where
lines are left out between two, `│ … N more lines` for those after the last.
Then `log <path>`, and another failed shard or run, or `log is only in the
remote cache`.

- **an action's excerpt:** the lines its ties quote, then what `std.excerpt`
  picks of its own log. With neither, its `failure_detail` message, unless its
  category and code say it is only Bazel's failed command (`COMMAND_FAILURES`:
  `spawn` with `non_zero_exit`, `timeout` or `out_of_memory`; `sandbox` with
  `subprocess_start_failed`). So a test Bazel failed itself quotes
  `Sharding requested, but the test runner did not advertise support for it`.
- **a test's excerpt:** its chosen attempt's `test.log`, Bazel's preamble left
  out, at most two lines of a timed-out one.
- **a target's excerpt:** Bazel's description, a Starlark `Traceback` cut as
  the console's recap cuts one, then the lines its ties quote. A target that is
  its own root cause with no description says `its package has an error`.

Past `--console:recap-max` entries (10 by default; `failure_summary_max`), the
heading says `Failures (first 10 of 14)` and the rest are counted:
`… and 4 more failures · aspect history show <id>`.

### Console ties

Bazel's console adds to an entry only where a line ties to it exactly
(`console_ties`), and only once the build has ended, from the collector's
blocks. A tie that takes "the one" line below needs exactly one line to fit
(`_tie`): where several fit, as when two actions failed alike, the entry gets
none, never a guess. A header is Bazel's when Bazel coloured it, unless it
coloured none (`--color=no`). An `INFO: From` block is a tool's output, never
Bazel's, and so are the blocks after it that are the rest of a successful
action's output (`_absorb_tails`, which needs every action's events).

| Entry | Tie | Gives |
|---|---|---|
| A sandboxed action | The one header whose text after `failed: (<status>): `, with what follows it, is exactly its `failure_detail` message, a newline and its own output as Bazel relays it (stderr, then stdout where the console carried Bazel's stdout); past the relay cap, Bazel's line saying it skipped that very file | Location, status |
| A local or worker action | The one header naming `target <label>)` with `(Exit <code>)` whose next lines are exactly its own output | Location, `exit N` |
| An action whose output was not created | The one `output '<path>' was not created` (or `declared output '<path>' was not created by genrule`) line for its primary output | The line, quoted; location |
| Every action of a target | Every header naming `target <label>)`, where they all give one location | Location |
| A root whose analysis or loading failed | The one header with a location whose text is its description's first line (also after `Skipping '<pattern>': ` and Bazel's `Failed to parse target pattern even though …: ` for a pattern) | Location |
| A target named by its own error | Each `<label>: …` line (an attribute error) | The lines, quoted; location where they agree |
| A target that failed with no action of its own | The one line ending `<label> failed: <message>`, else the one ending `failed: <message>` | Location |
| A root cause no line ties | The one line ending `and referenced by '<target>'` of the first target naming it alone | The line without that end, quoted; location |

Some lines are claimed without being quoted, so they are not listed again: a
line restating an action's message at its location;
`package contains errors: <pkg>: <text>` where an entry of that package quotes
`<text>`, or, with no text, says its package has an error;
`… <label> failed: …` and `Analysis of target '<label>' … failed` for a listed
label; and `package contains errors: <pkg>: Traceback …` where a block listed or
quoted already has that `Traceback`.

### Other errors from Bazel

Every `ERROR:` block no tie claimed, but Bazel's closing lines (`Build did NOT
complete successfully` and the like) and any inside a tool's output, is listed
once under `── Other errors from Bazel ──` (`_other_errors`): its line and the
lines Bazel wrote with it (a `Traceback`'s frames), the workspace's path left
out, capped as the entries are. A broken BUILD file has no event, so its errors
land here. What the collector's caps left out is counted on a line of its own.

### The counts line

```
193 built, 1 failed, 2 blocked · 47 tests passed, 2 failed · Bazel's raw log: <path>
```

`built` is each target whose `TargetComplete` succeeded; `failed` each label
with an entry that is no test's and no pattern's; `blocked` the blocked targets;
the tests' counts are the Tests row's (`summary_counts`). It starts with
`interrupted · ` where `BuildFinished` says `INTERRUPTED` or the task was
cancelled.

### The Failures tab

The tab has a row per entry, in the summary's order and with its heading
(`failure_rows`, `listed_failures`), a flaky test's too, marked `~` and never
counted. The chosen row shows its excerpt under it (every row's with Ctrl+O).
After them come the rows of no record: a failed target with no reason
(`✗ <label>  no log`), Bazel's exit code, each blocked target
(`○ <label>  blocked by :x`), and, once the build has ended, Bazel's other errors
(`! …`). The tab's badge (`Failures ✗6 ○1`) and the Results pane's `Failed`
row count the entries but a flaky test's, and the blocked targets. While the
build runs the entries have no console ties, so no location: those come when it
ends.

## Known limits

**Runfiles middlemen on Bazel 7 and 8.** Those versions leave a runfiles
middleman they evaluate out of `[n / m]`, but count it among the action cache's
hits, or, when it ran, among `internal`, with no count of either per mnemonic.
So `local` (or `internal`) includes one for each test or binary target Bazel
checked again, and the parts add up to more than N. Bazel 9.3.3 counts every
case below right.

| Trigger | Bazel 8.9.0 | Bazel 7.7.3 |
|---|---|---|
| A `test` build after a server restart, the action cache intact | 57 parts of `[52 / 52]` | 63 parts of `[58 / 58]` |
| `test //tx:tnoshard` with a new disk cache | 8 parts of `[7 / 7]` | 8 parts of `[7 / 7]` |
| `test --nokeep_going` after a coverage run | 26 parts of `[25 / 26]` | 29 parts of `[28 / 29]` |

**Internal actions on Bazel 7 and 8.** Their `runner_count` runners, spawns and
`internal` together, add up to `actions_executed`, so where some actions have
more than one spawn, `internal` falls short by as many, and those internal
actions are in `○`. A cold `test` of the same targets ends with `○ 8` on 7.7.3
and 8.9.0, whose runners add up to their 58 and 52 executed actions, and with no
`○` on 9.3.3, whose runners add up to 73 for 62 executed, 37 of them internal.
The `?` overlay says so (`_REMAINDER_NOTE`).

**Actions up to date in memory have no part of their own,** on any version: they
share `○` with actions an identical action made, and once something but a test
has failed, with actions whose input failed. Bazel 7 publishes no
`build_graph_metrics`. On Bazel 8 and 9, `n - built - cleaned` matches them on a
warm rerun (`[62 / 62]`, 10 built, 3 cleaned: 49) but not after `--nokeep_going`
or across new targets, and a Bazel 8 `test` build has more built than `[n / m]`
counts (58 of `[52 / 52]`), so the split does not use them.

**A test Bazel fails itself makes `○` say `skipped`.** Bazel exits
`BUILD_FAILURE`, not `TESTS_FAILED`, so the `○` part says `skipped` though its
actions are up to date: `○ 49 skipped` of `[62 / 62]` on a warm 9.3.3 rerun with
a sharded test whose runner ignores sharding. `skipped` holds everything
`up to date` does, so it is kept, with no test-only rule.

**A failed action whose spawn created its first output** is keyed in the
execution log by that file's id, not its path, so it can't be matched to its
failure: it can count as both `ok` and `failed`, its parts then past N. No
recording keeps an execution log, so the replay does not cover it.

**A tool's held last line in Bazel's colours.** Without every action's events,
a successful action's last line with no newline, which Bazel writes after its
next redraw, cannot be told from Bazel's next message where it starts with
Bazel's coloured `ERROR:`, and is listed under Other errors. That is every build
of the plain console unless your own flags or `ArtifactUpload` ask for every
action, and a Dashboard build with `--dashboard:actions=failed`, a
`--bes_backend` or a remote cache Bazel uploads events to.

## How it's verified

| Suite | What it checks |
|---|---|
| `aspect dev test-failure-replay` (`feature/failure_replay_test.axl`) | Every recorded failure through the summary and the Failures tab, against its golden; the counts and the parts' sum |
| `aspect dev test-failures` (`private/lib/failures_test.axl`) | One synthetic case per record, tie and entry rule |
| `aspect dev test-dashboard-actions` (`feature/dashboard/actions_test.axl`) | The split, the parts, the `○` words, the hit rate, the bar and the `?` overlay, against real Bazel 9.3.3 builds' `BuildMetrics` (`_REAL`) and live event sequences; every row as wide as the screen from 40 to 200 columns |
| `aspect dev test-dashboard` (`feature/dashboard_test.axl`) | Layout, keys and golden frames (`crates/aspect-cli/tests/testdata/dashboard`) |

CI runs every `aspect dev test-*` suite through `tools/run-axl-suites.sh`, and
`test-dashboard` and `test-failure-replay` again with `CLICOLOR_FORCE=1`.

### The failure fixtures

`crates/aspect-cli/tests/testdata/failures` holds 47 failure shapes recorded on
Bazel 7.7.3, 8.9.0 and 9.3.3, 192 recordings with their variants (`noall`
without `--build_event_publish_all_actions`, `nocolor`, `merged`, `timestamps`,
`console_only`); its README gives the layout. For each recording the replay
reads the events as the runtime reads a `--build_event_json_file`, runs each
through `collect_event`, takes the collected errors from the recorded console
(`bazel.output._collected_errors`) and Bazel's last `[n / m]` from it, then
requires:

- the summary's `Failures (N)`, the tab's `✗` rows and the entries to agree;
- the summary's other errors and the tab's `!` rows to agree;
- a passing build to have no failures;
- the actions' parts to add up to Bazel's last `[n / m]`, never more or less,
  where the recording has `BuildMetrics`.

Each recording runs after `bazel clean` with `--disk_cache=`, so its actions run
again; the recordings keep no execution log, so the split there is
`runner_count`'s.

To record one case, or every case, then rewrite the goldens and review the
diff:

```sh
tools/failure-fixtures/record.sh xn 9.3.3
aspect dev test-failure-replay --update --case=xn

tools/failure-fixtures/refresh_all.sh 7.7.3 8.9.0 9.3.3
aspect dev test-failure-replay --update
```

`record.sh` runs Bazelisk under `env -i` with a throwaway HOME and
`--nosystem_rc` (`env.sh`), its stderr on a 120-column pty. `fxpost.py` replaces
machine paths with placeholders (`${OUTPUT_BASE}`, `${WORKSPACE}`, `${HOME}`, …)
and drops the events the summary never reads. Its leak guard (`guard`) refuses a
recording that still names this machine's host (short or full), user or home as
a whole word or a path component, or Bazel's `_bazel_<user>`: `record.sh` deletes
it and exits 3. `FIXTURE_GUARD_ALLOW` lets a name that is an ordinary word stand
in text, and a fixture's own package names may stand as path components.
