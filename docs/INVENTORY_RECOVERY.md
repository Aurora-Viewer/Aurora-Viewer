# Inventory read recovery

This task follows [#3853](https://github.com/Aurora-Viewer/Aurora-Viewer/issues/3853),
[#3031](https://github.com/Aurora-Viewer/Aurora-Viewer/pull/3031), and
[#3027](https://github.com/Aurora-Viewer/Aurora-Viewer/issues/3027).

The recovery policy is implemented at the owners of the work, not by increasing
the number of retries in each UI producer. A successful read must win over an
obsolete failure, and an unresolved read must not be described as a deletion.

## Single-item reads

`session::inventory_reads::Reader` belongs to one login session. Producers put
UUIDs into one shared queue; repeated `(owner, UUID)` requests share an entry.
One worker admits at most 16 AIS item reads, including their response bodies.
Ordinary and attachment consumers retain their different delivery events.

After a folder/tree reply is applied, the main-thread model acknowledges only
originals it actually holds. This cancels redundant ordinary item requests,
including queued work and in-flight GETs. A raw HTTP snapshot cannot bypass
the model's folder-version checks. Attachment metadata requests keep their
separate parent-repair behavior. Unique request generations prevent a late
cancelation callback from failing a newer request for the same key.

After an exhausted read, a new request for that key waits for the shared
2/10/60-second cooldown. This prevents producers from independently restarting
the same failed read each frame. Healthy keys continue to load. The queue stores
one record per unique requested key, rather than spawning a task per command;
its unique-key count is not capped by an arbitrary inventory-size limit.

If a producer supplies a newer capability while a read is active, the older
result is discarded and the same waiting consumers are read on the new
capability. Dropping the reader cancels its worker and the worker's read tasks.

Folder/tree reads retain their existing scheduling budget. This is a bound on
single-item reads, not a claim that all HTTP requests in the viewer share one
connection limit. The server's own throttling and availability still matter.

## Folder retries

Automatic folder requests must consult the background cooldown ownership.
They only enqueue Unknown folders; a Failed or over-limit folder is not
restarted by automatic UI demand. The existing once-per-region recovery uses
an explicit retry method, while manual refresh re-arms the model deliberately.
A due retry must re-check each target's current state: recovered targets are
walked for descendants without refetching; externally active targets wait;
deleted or over-limit targets are not reset to Fetching. Partial recovery of a
subset must leave healthy children intact.

## Existence and diagnostics

HTTP 404/410 and explicit valid legacy missing-item replies can establish that
an original is absent. Transport failures, 5xx, malformed LLSD and incomplete
records cannot. Exhausting a local retry budget leaves an item unavailable,
without adding it to the confirmed-missing set. Outfit loading may still time
out normally; it must not silently classify the unavailable original as a
deleted link target.

Single-item AIS reads use the timed and paced AIS read path. Diagnostics retain
status/failure kind and parsing-stage reasons, without capability URLs or
response bodies. Successful later reads clear local retry bookkeeping, and
late failures for an already-present item are ignored.

## Validation

Regression tests cover cooldown bypass, obsolete retries after recovery, a
partially recovered subset, duplicate producers, shared concurrency, session
cancelation, replaced capabilities, transient service recovery, and the
distinction between failed reads and confirmed absence. Test evidence is kept
with the task; only commands actually run are reported as passing.

The final offline run of `check.ps1` passed its PowerShell parser check,
format check, Clippy with warnings denied, and locked workspace tests. The
network crate passed 568 tests; the viewer passed 8,104 (18 ignored by the
existing suite). Its large-inventory regression recovered all 240,200 items
after ten injected transient batch failures. This is a simulated server, not
a throughput measurement on a real account. Six isolated Rust folder-recovery
regressions fail on the original method bodies and pass on the corrected ones.

Live validation must compare cold/warm login and Appearance closed/open on a
large real account, including time to a usable outfit and final inventory
counts. Offline tests cannot prove a particular server's completion time or
that every previously failing private inventory record is readable.

Separating texture/inventory cache clearing, periodic cache checkpoints,
cross-kind global HTTP fairness, and response-memory budgets remain separate
improvements. None substitutes for correctness of the read state machines.
