# NeQuick G reference data

**Status: staged, not wired up. No code in this repository reads these files.**

They are here so that a future NeQuick-G ionosphere backend has its reference
data already vendored and already licensed, and so the provenance is recorded
while it is still verifiable. Until that backend exists, the app's ionosphere
comes from `app/src/chapman.rs`, `app/src/fof2.rs` and `app/src/sporadic_e.rs`,
and from the bundled grid in `app/src/assets/fof2_grid.tsv`.

Do not confuse the two. The bundled `fof2_grid.tsv` is **not** derived from
these files. Its header says so, and so does `app/src/bin/iono_check.rs`: its
layout is the operational one but its values come from the app's own
order-of-magnitude climatology. It is not CCIR, not URSI and not IRI.

## Contents

| File | What it is |
|---|---|
| `ccir11.txt` ... `ccir22.txt` | Monthly CCIR coefficient maps. The number is the month plus 10, so `ccir11` is January and `ccir22` is December. Each file holds 2858 values: 1976 foF2 coefficients as `F2[2][76][13]`, then 882 M(3000)F2 coefficients as `Fm3[2][49][9]`. The leading dimension is the low/high solar activity pair, R12 = 0 and R12 = 100. |
| `modipNeQG_wrapped.txt` | Modified dip latitude grid, 39 x 39 = 1521 values in degrees. A 5 degree latitude by 10 degree longitude core, wrapped at the poles and in longitude so a third-order 4 x 4 interpolation never needs a special case at the edges. Derived from IGRF around epoch 2001. |
| `_SOURCE_AND_LICENSE.txt` | Provenance and licence. Required to stay with the data. |

## Licence

EUPL v1.2, (C) European Union 2019. This is **not** the GPL-3.0 that covers the
rest of the repository; it is one of the exemptions the top-level `LICENSE`
refers to. Article 5 of the EUPL requires the notice to travel with the data, so
`_SOURCE_AND_LICENSE.txt` must not be deleted or moved away from these files.

Extracted from the Annex C attachments of the European Commission's "Galileo
Ionospheric Correction Algorithm" ICD, Issue 1.2, September 2016. Byte-identical
to the NeQuickG JRC reference implementation. The CCIR values agree with
ITU-R P.531-16 to 5.6e-7 relative.

## If you are removing this

Delete the whole `data/nequick/` directory in one go. Keeping the coefficient
files without `_SOURCE_AND_LICENSE.txt` would breach the licence.
