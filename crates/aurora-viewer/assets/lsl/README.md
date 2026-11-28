# LSL definitions

`definitions.json` is the table the script editor reads from
[secondlife/lsl-definitions](https://github.com/secondlife/lsl-definitions) —
MIT, Copyright (c) 2025 Linden Research, Inc.; the licence text is in
[LICENSE-lsl-definitions.txt](LICENSE-lsl-definitions.txt). It holds:

- `deprecated`: the name of every deprecated function, constant, event and
  control, and what to use instead (`null` when it has no replacement);
- `arguments`: the parameters of every function and event, in signature order,
  each with the definitions' name and what it means (an empty meaning for the
  handful the definitions leave undescribed).

It was made from release `v0.7.2.1-die-no-rezzer` (commit
`a4be83c390aa60cef5f7066cd2fcb7ee9823eaf6`), `lsl_definitions.yaml` (SHA-256
`c3a8dbfbddd110802996d867db546da1ed5251e5af23c4dfeadf3f6c5a37e917`), reading only
its `deprecated:` and `arguments:` blocks, by
[generate-lsl-definitions.py](../../../../scripts/generate-lsl-definitions.py).
To refresh it, change the release, commit and hash in that script and run it;
the file records the source it was made from.

The definitions are not what the viewer compiles against: the region's LSL
syntax stays the source of names and types (`reference.json` here, and
`aurora-net`'s `lsl_syntax`). The table only decorates names the region already
knows, so a name it lists that the region lacks is never offered. A signature's
parameters take the definitions' names (`channel`, `msg`, as the wiki writes
them) when both agree on how many there are; a region newer or older than the
table keeps its own names and gets no help rather than the wrong one. A
replacement is written as the definitions write it, `ll.Name`; the LSL help
shows `llName`, the Lua help `ll.Name`.
