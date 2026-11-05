# Debatable module homes (temporary; deleted once the owner decides)

- `grid.rs` went to skipzone-iono; alternative skipzone-lab. Only lab (corpus, wspr, WSPR harnesses) and app (state/location) use it, but it is a pure geographic conversion with no I/O, and the app may not depend on lab.
- `clock.rs` went to skipzone-iono; alternative skipzone-predict. Nothing in the physics reads it (solar takes the date as input) and only the app's state/ui and one harness use it, but it is a dependency-free conversion that sits naturally beside `solar`. Note it reads the system clock in `utc_now`.
