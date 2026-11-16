# Large-inventory recovery: human validation and sanitized evidence

The reporter manually tested candidate `d56180959dba9e2783206624281e922650ca1888`
(`Aurora Viewer 0.9.10+5.gd56180959`) on 2026-11-15 local time
(2026-11-16 UTC). They report a very smooth inventory experience and few or
no failures visible in the monitor. Four local sessions confirm background
completion on the candidate. Grid validation was performed by the reporter.

The reporter subsequently tested final candidate
`5ce37919de8c837d7489425984f2c874d7f7f301`
(`0.9.10+27.g5ce37919d`) and confirmed successful inventory and attachment
restoration. Its zero-cache session T6 completes 17,367 folders / 219,982 items
in 133.4 s, with zero folders left and 37/37 attachments worn. All grid tests
were performed manually by the reporter.

This evidence supports recovery and responsiveness on a partial/warm cache.
The reporter then performed a genuinely cold inventory-metadata run (T5): its
log restores zero cached folders and reaches background completion in 146.1 s.
**A single cold run is not a controlled speedup benchmark or a latency guarantee.**

## Session comparison

| Session | Build | Cached folders restored | Background completion | Folder requests reported by background fetch | Final folders/items | Failed-item result notifications |
|---|---|---:|---:|---:|---|---:|
| B0 | 0.9.10 | 0 | No completion line in the captured session | Not available | Not established | 265,963 |
| T1 | candidate d561809 | 11,355 | 87.2 s | 2,295 | 17,367 / 219,975 | 0 |
| T2 | candidate d561809 | 17,343 | 17.6 s | 2 | 17,367 / 219,975 | 0 |
| T3 | candidate d561809 | 17,328 | 18.0 s | 3 | 17,367 / 219,975 | 0 |
| T4 | candidate d561809 | 17,309 | 17.1 s | 4 | 17,367 / 219,976 | 0 |
| T5 | candidate d561809 | 0 | 146.1 s | 2,909 | 17,367 / 219,980 | 0 |
| T6 | final candidate 5ce3791 | 0 | 133.4 s | 2,909 | 17,367 / 219,982 | 0 |

Every candidate completion reports zero folders left and zero folders over the
server limit. These are the background phase's times and counts, not end-to-end
login times or an independent server-side inventory audit. The account is
approximately 240k when folders and items are considered together. Small count
differences between sessions can reflect normal updates; no deletion claim is
made. Background request counts do not include all single-item HTTP attempts.

B0's notification count includes repeated IDs. It also has 726 folder-failure
and 166 background-failure results; the candidate sessions have zero such
terminal notifications. B0 and T1-T4 did not start from the same cache state or
at the same time. No failure-rate percentage or speed multiplier is inferred.

## 503 responses still occurred and recovered

The debug-enabled candidate sessions T1, T3, T5 and T6 recorded 147, 48, 421 and
353 HTTP 503 retry messages respectively. T1 has 108 messages at retry 1 and 39
at retry 2; T3 has 44 at retry 1 and 4 at retry 2; T5 has 266 at retry 1 and
155 at retry 2; T6 has 273 at retry 1 and 80 at retry 2. None has exhausted-read
warnings or final item/folder failure notifications. Numeric inventory-only
HTTP profile segments independently record 147/48/421/353 failed 5xx attempts.

The absence of debug retry messages in T2/T4 is **not evidence of zero 503s**:
those sessions use a different logging level. The monitor's failed field is a
sampled-period counter. A screenshot showing zero cannot establish that the
whole session had zero failed HTTP attempts. The generic `retries=0` probe
field also does not count the AIS helper's internal retry loop; the explicit
status retry messages are used here instead.

HTTP 503 means the HTTP service is temporarily unavailable, often because of
overload or maintenance ([RFC 9110, 15.6.4](https://www.rfc-editor.org/rfc/rfc9110.html#name-503-service-unavailable)).
An origin service or an intermediary can emit the response. Excessive client
work can contribute load, but client logs alone cannot identify the inventory
backend's exact reason, distinguish a gateway refusal from an origin refusal,
or prove that this viewer caused the service overload. That needs service-side
diagnostics/request correlation. A timeout with no HTTP response is a different
observation and must not be counted as a 503.

## Why the UI cache-clear trials were warm

T1 records cache clearing after background completion; T2 subsequently restores
17,343 folders. T2 clears again, and T3 still restores 17,328 folders.
`DiskCache::clear` does cover the inventory store, but it does not clear the
inventory model in memory. Normal logout/exit calls `save_inventory_cache`,
which can recreate the file. Consequently, clearing while logged in and then
exiting does not guarantee a cold next login.

A controlled cold inventory-metadata test must close the viewer first,
preserve/move the inventory store, then manually log in and verify
`inventory cache: 0 folders up to date` in that session's log. Texture cache can
remain warm; that is a cold inventory test, not an all-assets-cold test. Do not
change the configured cache directory for this purpose: existing startup
behavior removes the previous cache when the location changes.

T5 follows this procedure and verifies zero restored folders. Its background
phase completes at process-relative +167.873 s (146.1 s of background work).
The separate local integration build was running during this human test, so
the timing is descriptive rather than a clean performance benchmark.

## Cooldown cost and boundaries

Ordinary first reads have no added delay. The AIS helper uses up to five
retries after the first attempt, normally paced 1/2/4/5/5 seconds; a numeric
`Retry-After` can extend a delay up to 60 seconds. A new per-key 2/10/60-second
cooldown applies only when a logical item read has exhausted/failed and a
producer asks for that key again. It does not pause healthy keys or the whole
inventory. Success clears the bookkeeping. The original background-folder
15/60/300-second backoffs remain unchanged; the patch makes other automatic
callers respect their owner instead of bypassing it.

The candidate sessions have no terminal item-read failures, so they do not
show a latency cost from the new per-key cooldown. Under persistent failures
it can delay a particular item's next attempt; this is a deliberate tradeoff
against duplicate storms, not a guarantee of zero latency for failing items.
Folder/tree reads still have a separate budget. No global adaptive HTTP
controller or service-side fix is claimed.

## Evidence handling

`comparison.json` contains aggregates only. The seven `*-excerpts.txt` files
contain a strict allowlist of inventory startup/cache/completion messages,
numeric inventory HTTP segments, and at most a few sample failure messages.
Times are relative to process start. Account identifiers, UUIDs, locations,
hostnames, capability URLs, local paths, payloads and unrelated text were not
copied. Whole raw logs and original screenshots are not committed.

The reporter supplied before/after monitor screenshots. Their numeric
observations are 1,584 versus 1 active inventory reads, 543 versus 0 failures,
0/s versus 37/s completion, and 2,317.0 versus 60.9 ms median latency. These
are unmatched snapshots, not a benchmark. With the reporter's permission, only
the HTTP tables are published as lossless crops on the fork's captures branch;
the originals retain their private region/background UI. Each crop was checked
pixel-for-pixel against its source rectangle; no numbers were edited or
rescaled. The images are illustrative inventory-monitor samples, not evidence
of the final attachment state or of zero errors throughout a whole session.

### Cropped HTTP monitor samples

Before: 1,584 active inventory reads, 543 failed attempts in the sampled period,
0 completions/s, 2,317.0 ms median latency.

![Before inventory recovery](https://raw.githubusercontent.com/SmurfCat1234/Aurora-Viewer/04a5c198217d8052f87f35b8f69e584c8eb2dd2a/inventory-recovery/http-before.png)

After the inventory-recovery candidate: 1 active read, zero failed attempts in
that sampled period, 37 completions/s, 60.9 ms median latency. These unmatched
samples illustrate the monitor behavior, not a controlled benchmark or a claim
that the complete session had no transient errors.

![After inventory recovery](https://raw.githubusercontent.com/SmurfCat1234/Aurora-Viewer/04a5c198217d8052f87f35b8f69e584c8eb2dd2a/inventory-recovery/http-after.png)

## Attachment restoration before/after

The inventory-only candidate T5 starts with 37 COF object targets, waits about
15 seconds for missing originals, then drops ten unknown targets as broken
links. It later reports 27/27 worn, despite starting with 37 targets. This is
an existence/timing error in the outfit restore, independent of the inventory
background phase eventually completing. The log does not prove every visible
missing object had this single cause.

Final candidate T6 starts with the same COF version and 37 targets. It retains
17 still-loading originals at the grace-period boundary, restores available
objects, then sends late wears in batches of 1, 1 and 3 as records arrive.
At process-relative +142.145 s it reports **37/37 attachments worn**; background
inventory completes at +201.229 s. The reporter confirms successful behavior.
`attachment-before.json` and `attachment-after.json` retain only numeric,
allowlisted messages with relative timestamps, never item names or IDs.

The 150-second value in one regression test is simulated late data, not a
production waiting period. Pending targets are checked during updates; metadata
availability, accepted COF intent, region readiness and existing settling rules
determine when a wear may be sent. Waiting does not create new GETs. Explicit
detaches, accepted unlinks, confirmed absence and non-attachable targets cancel
the pending intent.

## Still needed

- Additional cold/warm repetitions and an explicitly recorded Appearance
  closed/open comparison; T5 already establishes one zero-cache completion.
- Region transition/reconnect and explicit retry checks on the large account.
- The reporter has confirmed final candidate 5ce3791 and authorized submission.
  Material code changes after that still need appropriate validation.
- Maintainer review/CI. No claim that all server 503s, genuine missing records,
  or every possible large-inventory failure is eliminated.
