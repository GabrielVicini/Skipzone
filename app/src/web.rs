//! Web entry point: the browser's equivalent of `main.rs`.
//!
//! This is a proof of concept, not the primary target - Windows is, and the
//! native binary is unaffected by everything here. The whole module is compiled
//! only for wasm32.
//!
//! # What is different in the browser
//!
//! Two things, both consequences of wasm32-unknown-unknown having no threads:
//!
//! * There is no solver worker thread, so a job runs on the browser's main
//!   thread and the tab is unresponsive until it finishes (see
//!   [`crate::sweep::SolverService::dispatch`]). A single trace is quick; a
//!   frequency sweep or a coverage grid is not, and its progress arrives in one
//!   batch at the end rather than streaming in. CANCEL cannot interrupt a job
//!   that has already started.
//! * The compute pool is forced sequential (see [`crate::compute::ComputePool`]),
//!   so there is no parallel speed-up. Results are bit-identical either way,
//!   which is what makes the substitution safe.
//!
//! Everything else - the engine, the models, the map, the coastline data - is
//! the same code the native build runs.

use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

use crate::app::SkipzoneApp;

/// Id of the `<canvas>` in `index.html` that the app renders into.
const CANVAS_ID: &str = "skipzone_canvas";

/// Boot the app onto the canvas. Called from `index.html` once the wasm module
/// has loaded.
///
/// # Panics
/// If the document has no canvas with the expected id, which would mean the
/// shipped `index.html` and this module have gone out of step.
#[wasm_bindgen]
pub fn start() {
    // Rust panics reach the browser console as readable messages rather than
    // "unreachable executed", which is the difference between a debuggable web
    // build and an opaque one.
    console_error_panic_hook::set_once();

    let canvas = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(CANVAS_ID))
        .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .expect("index.html must contain a canvas with id skipzone_canvas");

    wasm_bindgen_futures::spawn_local(async {
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(SkipzoneApp::new(cc)))),
            )
            .await;
        if let Err(e) = result {
            web_sys::console::error_1(&format!("skipzone failed to start: {e:?}").into());
        }
    });
}
