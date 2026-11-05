# Debatable module homes (temporary; deleted once the owner decides)

- `grid.rs` went to skipzone-iono; alternative skipzone-lab. Only lab (corpus, wspr, WSPR harnesses) and app (state/location) use it, but it is a pure geographic conversion with no I/O, and the app may not depend on lab.
- `clock.rs` went to skipzone-iono; alternative skipzone-predict. Nothing in the physics reads it (solar takes the date as input) and only the app's state/ui and one harness use it, but it is a dependency-free conversion that sits naturally beside `solar`. Note it reads the system clock in `utc_now`.
- `calib.rs` went to skipzone-predict; alternative skipzone-lab, as first planned. It holds the model's runtime parameter set (`Anchors`, `AtmosphericAnchors`, `Bounded`), which `scenario` and `noise` read at runtime, so predict cannot do without it; only the fitting that moves the anchors belongs in lab.
- `coastline.rs` went to skipzone-predict, with the two Natural Earth shapefiles; alternative skipzone-app. The map draws coastlines, but the ground-reflection loss needs the land mask (`scenario::GroundType` from the point's surface), so the headless solve needs it too.
