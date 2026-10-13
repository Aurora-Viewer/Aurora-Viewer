# V-HACD 4.1

`VHACD.h` and `LICENSE` come from the public tag `v4.1.0`, revision
`22ec20a7f8ea221ab600df869369b9c6a258cb10` of the
[V-HACD repository](https://github.com/kmammou/v-hacd/tree/22ec20a7f8ea221ab600df869369b9c6a258cb10).
SHA-256 of the original file:
`02314c575909d656dc67d6e3c38cefc49c4403e8ca1dcb0ab4b05800cee61e1c`.

The version is pinned to keep the Flood, Surface and Raycast modes and the
analysis parameters used by Firestorm. The header is kept unmodified under
its BSD-3-Clause licence. `bridge.cpp` is the Aurora adapter: ownership of
the allocations, contained exceptions, cancellation and progress.
The models are processed in parallel by the Rust workers; each native
instance runs synchronously to avoid multiplying threads.
