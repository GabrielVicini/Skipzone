# Documentation

Every document here either documents a crate or is a dated report. How the
project fits together, how to use the app and how to build it are in the
[wiki](https://github.com/GabrielVicini/Skipzone/wiki). The API reference is
`cargo doc --workspace --no-deps --open`.

## Derivations

The physics each crate implements, derived before the code was written. Each
file opens with the functions that implement it and the tests that check it,
and the code cites it by section, for example
`docs/derivations/engine/haselgrove.md §2`. Written for anyone changing the
physics or checking that the code does what it claims.

| Document | Crate | What it derives |
|---|---|---|
| [conventions.md](derivations/conventions.md) | all | Coordinates, the $e^{-i\omega t}$ time convention and the sign of losses, units, and the symbol table the other files use. Read it first. |
| [engine/haselgrove.md](derivations/engine/haselgrove.md) | `skipzone` | The Haselgrove ray equations in spherical coordinates, the group, phase and absorption integrals, launch on the dispersion shell, and conditioning at reflection. |
| [engine/appleton-hartree.md](derivations/engine/appleton-hartree.md) | `skipzone` | The complex Appleton–Hartree refractive index from the electron equation of motion, mode labels, the stable forms the code evaluates, degeneracies and analytic partials. |
| [engine/magnetic-field.md](derivations/engine/magnetic-field.md) | `skipzone` | The IGRF field and its coordinate Jacobian from the scalar potential, the Schmidt Legendre recurrences, the dipole reference and the divergence and curl checks. |
| [engine/analytic-solutions.md](derivations/engine/analytic-solutions.md) | `skipzone` (tests) | The reference answers the field-free validation suites compare against: the Bouguer invariant, quasi-parabolic closed forms and the Bouguer quadrature. |
| [iono/chapman-grazing.md](derivations/iono/chapman-grazing.md) | `skipzone-iono` | The Chapman grazing-incidence function, the scaled error function, the horizontally varying Chapman layer and its gradients, the night floor and the partial rise. |

`skipzone-predict`, `skipzone-lab` and the app have no derivations of their
own. Their models are documented in rustdoc next to the code.

## Reports

Dated snapshots. They are not maintained, and their numbers describe the
commit they name, which may differ from the current code.

| Report | What it covers |
|---|---|
| [2026-08 verification](reports/2026-08-verification.md) | Engine correctness, ionosphere against GIRO ionosonde data and end-to-end SNR against WSPR, at commit `d92bf9f`. For anyone weighing how far to trust a prediction. |
| [Accuracy roadmap](notes/accuracy-roadmap.md) (note, 2026-11) | A held-out benchmark against ITU-R P.533, then candidate accuracy improvements with their data sources, licences, effort and expected effect, for choosing what to build first. |

## Keeping this honest

`scripts/check-docs.py` runs in CI. It fails if a derivation is not cited by
any code, if a citation in code names a section or equation the file does not
have, if a derivation's front matter names code or tests that do not exist, if
a document here is missing from this index, or if markdown appears outside
`docs/`, `wiki/` and `.github/` without a reason recorded in the script.
