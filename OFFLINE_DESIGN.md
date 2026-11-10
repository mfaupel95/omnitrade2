# Offline execution proof — approved scope

Fork verified via GitHub API: mfaupel95/omnitrade2, fork of jackby03/omnitrade.
origin/HEAD verified by ls-remote: d60b6091968f93f5dcd6ed007bbd5a0eec44a805,
default branch hermes/okx-reconnect-20261021. Operating checkout HEAD is
3308c2a124b1b01fac22e7bfbaa391605b9b0f16, not the fork default: results must
not be represented as a rebuilt operating binary.

Use existing live_loop::place_order_guarded directly, not a reimplementation.
It already contains the shared execution seam; no production extraction needed.
A stateful test-only FakeExchange implements ExchangeTrading and applies actual
returned fills to signed position and cash. Tests call the same guarded function
as live_loop. Historical replay calls compile_and_run on the same 500-candle
sliding buffer, sends each non-Hold as a 0.1 market step to that function.

TDD: first add a failing stateful-fill test with an inert fake, observe failure,
then implement fake accounting. Add boundary and error coverage against existing
live behavior; existing behaviors are characterization tests, not new features.
Fake prices, full/partial fills, fees, rejects, unavailable account/position are
explicit model assumptions. No broker, credentials, network or real orders.
No stop is implemented or inferred from stop_prozent; no liquidation/margin,
contract rounding or funding parity claimed. No sweep, push, merge or restart.

Build from current source in private target, offline Cargo, debug info disabled,
limited jobs. Record actual logs, input hashes, results and unverified gaps.
Only this worktree is writable; original checkout and other worktrees read-only.
