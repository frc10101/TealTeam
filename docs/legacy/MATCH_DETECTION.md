# MATCH DETECTION

> **Historical.** A diagram carried over from `internal/handlers/MATCH_DETECTION.md` in the retired tree. That source file no longer exists, and this is all that survived of it.
>
> The live logic is `tt_core::matches` — `classify()`, `CURRENT_WINDOW`, `CompLevel`, and `label()`.

```mermaid
flowchart LR
	Source[Match source (TBA / FIRST / Scouts)] --> Detector[Match detection logic]
	Detector --> Action[Schedule / Assign / Display]
```
