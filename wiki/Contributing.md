# Contributing

The project is pre-alpha and moving. This page is about how to make a change
that will survive review rather than about process ceremony.

## Before you start

Read [Conventions](Conventions.md). Most of the rules there exist because
breaking one produced a wrong answer that looked right, and a change that
violates one will be asked to change.

## The check that has to pass

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets       # with RUSTFLAGS=-D warnings
cargo test --workspace --all-targets
cargo build --package skipzone --lib
cargo doc --package skipzone --no-deps       # with RUSTDOCFLAGS=-D warnings
```

That is exactly what CI runs. See [Building and CI](Building-and-CI.md).

## Changing the physics

**Derive it first.** Add or extend a file in `docs/derivations/`, then implement
it, then cite the section in a code comment. A pull request that adds an
equation with no derivation will be asked for one, and the derivation is the
part that gets reviewed hardest.

Then find the oracle. Every piece of physics in this project is checked against
something independent:

| Kind of change | Check it against |
|---|---|
| A new field model | The dipole closed form, plus divergence and curl invariants |
| A new density profile | A closed-form range/apex if one exists, otherwise the Bouguer quadrature reference |
| A new gradient | Finite differences, in a `*_matches_fd` test |
| An integrator change | The convergence-order test |
| A conditioning fix | The regime it fixes, plus the regime it must not break |

If your change has no independent check, say so explicitly in the pull request.
That is sometimes the right answer, but it should be a stated decision.

## Changing an anchor

Anchors are the unverified values in `app/src/calib.rs`. Changing one is
allowed. Changing one **because a fit wanted it** is not, unless the fit could
identify it.

Before moving an anchor:

1. Check the identification table in
   [Calibration](Calibration.md#what-that-makes-unidentifiable-on-purpose). If
   the quantity is in the "no, absorbed" column, WSPR cannot see it and a fit
   that moved it fitted something else.
2. Check whether the fit hit the bound. A bound being reached is a finding: the
   residual is not produced by the quantity being pushed, and the error is
   elsewhere.
3. Update the docstring's stated range and trust level along with the value.

`iono_check --propose` will print suggested changes. Applying one is a separate,
deliberate commit with its own test, not part of the run that suggested it.

## Changing performance

Use `solve_digest`.

```bash
cargo run --release -p skipzone-app --bin solve_digest > before.txt
# make the change
cargo run --release -p skipzone-app --bin solve_digest > after.txt
diff before.txt after.txt
```

An optimisation that skips work is only safe if it changes which rays are
*traced* and not which are *found*. An empty diff is that proof. A non-empty
diff needs an explanation of every line before the change lands.

`profile_solve` tells you where the time actually goes, which is usually not
where it feels like it goes.

## Changing the layer boundaries

The app's five layers depend downward only. Two invariants are greppable and
both should be checked after touching that area:

```bash
grep -rn "use skipzone::" app/src/ui app/src/state     # must be empty
grep -rln "egui" app/src | grep -v "app/src/ui/" | grep -v app/src/app.rs
```

The second should list only `app/src/main.rs` and `app/src/lib.rs` (a doc
comment). If a computation-layer module needs to tell the UI something, that is
what the `sweep::Wake` callback is for.

## Adding a dependency

To the **app**: fine, with a comment in `app/Cargo.toml` saying what it is for.
Every existing dependency there has one.

To the **engine**: this is a significant change and needs an argument. The
engine has two dependencies and a stated promise to stay buildable with nothing
beyond `std` on any target with no system libraries. CI enforces the build; it
cannot enforce the spirit.

A dependency was removed from the engine once already: `rayon` was there for a
single `trace_fan` function that only a benchmark called, while the app did its
own batching. If a new engine dependency serves one function that nothing in
production calls, it is the same mistake.

## Adding an antenna

Closed-form patterns go in `app/src/antenna/image.rs` with a `provenance` string
saying where the model comes from and what it excludes. That string is held to
the same standard as the existing ones, which cite NEC-4 comparisons.

Measured or NEC-modelled patterns go through `app/src/antenna/table.rs`, which
is built and tested for exactly this and is deliberately not wired up yet. The
module documentation has the four steps. Adding one should be a data file plus
an `AntennaKind` variant, not a redesign. If it turns into a redesign, that is a
bug in `table.rs`.

## Documentation

The wiki lives in `wiki/` in the repository so it versions with the code. A
change that makes a wiki page wrong should fix the page in the same commit.

Write for someone who is competent and unfamiliar. State what a thing is, then
what it is not, then why. The "what it is not" is the part that saves time.

Avoid em dashes.
