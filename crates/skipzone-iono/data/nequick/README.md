# NeQuick G reference data

**Status: in use.** `app/src/ccir.rs` compiles all thirteen files in with
`include_str!` and evaluates the CCIR foF2 map, which is the app's default F2
peak-density source. Measured against 70 219 GIRO ionosonde soundings it reads
foF2 to 1.10 MHz RMS against the previous model's 1.70; see
`wiki/Ionosphere-Model.md`.

The M(3000)F2 half of each file is parsed and evaluated but not yet consumed.

Do not confuse these with `crates/skipzone-iono/src/assets/fof2_grid.tsv`, which is **not**
derived from them: its layout is the operational one but its values come from
the app's own order-of-magnitude climatology. It is not CCIR, not URSI and not
IRI, it is still selectable as `Fof2Backend::Gridded`, and it is no longer the
default.

## Contents

| File | What it is |
|---|---|
| `ccir11.txt` ... `ccir22.txt` | Monthly CCIR coefficient maps. The number is the month plus 10, so `ccir11` is January and `ccir22` is December. Each file holds 2858 values: 1976 foF2 coefficients as `F2[2][76][13]`, then 882 M(3000)F2 coefficients as `Fm3[2][49][9]`. The leading dimension is the low/high solar activity pair, R12 = 0 and R12 = 100. |
| `modipNeQG_wrapped.txt` | Modified dip latitude grid, 39 x 39 = 1521 values in degrees. A 5 degree latitude by 10 degree longitude core, wrapped at the poles and in longitude so a third-order 4 x 4 interpolation never needs a special case at the edges. Derived from IGRF around epoch 2001. |
| `_SOURCE_AND_LICENSE.txt` | Provenance and licence. Required to stay with the data. |

## Licence

EUPL v1.2, (C) European Union 2019. This is **not** the MIT licence that covers the
rest of the repository; it is one of the exemptions the top-level `LICENSE`
refers to. Article 5 of the EUPL requires the notice to travel with the data, so
`_SOURCE_AND_LICENSE.txt` must not be deleted or moved away from these files.

Extracted from the Annex C attachments of the European Commission's "Galileo
Ionospheric Correction Algorithm" ICD, Issue 1.2, September 2016. Byte-identical
to the NeQuickG JRC reference implementation. The CCIR values agree with
ITU-R P.531-16 to 5.6e-7 relative.

## If you are removing this

Do not, without also removing `app/src/ccir.rs` and moving the default foF2
backend back to `Fof2Backend::Gridded` - the app will not compile otherwise.
If you do remove it, delete the whole `data/nequick/` directory in one go:
keeping the coefficient files without `_SOURCE_AND_LICENSE.txt` would breach the
licence.
