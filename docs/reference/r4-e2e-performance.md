# R4 end-to-end `journals process` performance

Plan 21 requires native `journals process` to use at least 4× less process
CPU than Python end to end. This record measures the integration stack's
test-only prepared-book pipeline ([R4-13 checkpoint](r4-journal-process-checkpoint.md))
on the four synthetic bench books. Production routing is unchanged.

## Method

- Books: the generic and multi-wallet 2,000 and 20,000 bench books, migrated
  by this tree's Python and copied fresh for every run.
- Python: the live `journals process` worker of this tree
  (`scripts/benchmark-journal-process.py`'s worker, prepared-book seam: no
  repairs or pricing), with this tree's tax-engine binding.
- Native: the `journal-process` example (`--release --features
  test-harness,timing`) on the same book copy, with the request, profile and
  UUID tape the Python run recorded.
- Eleven alternating Python/native pairs per book; the figure is the median
  process CPU time (user plus system, every thread) from `getrusage`. Phase
  figures are medians of the native CPU spans.
- Byte parity on each book before timing: payload and all 112 tables, REAL
  bits and row ids included, equal Python's.

## Integration stack at `06e253ea6` (2026-11-06)

Load average 13–81 during the runs (eleven workers, I/O-bound).

| Book | Python | Native | Ratio | Native range |
| --- | ---: | ---: | ---: | --- |
| generic 2k | 1.758 | 0.338 | 5.20× | 0.320–0.355 |
| multi-wallet 2k | 2.179 | 0.497 | 4.38× | 0.473–0.514 |
| generic 20k | 12.422 | 3.551 | **3.50×** | 3.504–3.710 |
| multi-wallet 20k | 20.866 | 4.733 | 4.41× | 4.656–4.839 |

Native phases (median seconds):

| Phase | generic 2k | multi-wallet 2k | generic 20k | multi-wallet 20k |
| --- | ---: | ---: | ---: | ---: |
| snapshot load | 0.029 | 0.054 | 0.286 | 0.407 |
| custody | 0.061 | 0.120 | 0.507 | 0.997 |
| render inputs | 0.049 | 0.054 | 0.511 | 0.557 |
| fusion request | 0.029 | 0.031 | 0.294 | 0.300 |
| engine boundary | 0.052 | 0.089 | 0.435 | 0.733 |
| render boundary | 0.022 | 0.029 | 0.222 | 0.252 |
| persistence and commit | 0.028 | 0.039 | 0.722 | 0.825 |
| drops | 0.041 | 0.052 | 0.403 | 0.451 |
| other | 0.048 | 0.059 | 0.393 | 0.464 |

Generic 20k misses 4× by about 0.45 s. Its largest movable costs are the
rendered JSON the pipeline builds and frees: the quantity stage and the
finalized projection as JSON trees (render inputs 0.51 s, most of the 0.40 s
of drops), and the request builder's key lookups in the rendered rows
(0.29 s). The engine's worker threads add CPU time to the engine boundary.
