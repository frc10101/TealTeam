//! Where the device's database lives between visits: `tealteam.sqlite3` at
//! the root of the origin private file system, where `snapshot.js` puts it.
//!
//! Through `navigator.storage` on whatever global this runs in, so the same
//! code serves a page, a worker, or the service worker (C5). Called by name
//! through `Reflect` rather than `web-sys`, whose file system bindings are
//! still behind `web_sys_unstable_apis`.

use js_sys::{Array, Function, Promise, Reflect, Uint8Array};
use tt_repo::{RepoError, Result};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use crate::ClientRepo;

/// The file's name, as `ttSnapshot.DB` has it.
pub const DB: &str = "tealteam.sqlite3";

/// Open the device's database, and save every change back to it.
///
/// [`RepoError::Unavailable`] when there is no OPFS here, which over plain
/// http is always, or no database yet, which wants a snapshot first (S10).
pub async fn open() -> Result<ClientRepo> {
    let dir = root().await?;
    let file = call(&dir, "getFileHandle", &[DB.into()])
        .await
        .map_err(|_| {
            RepoError::Unavailable("this device has no copy of the database yet".into())
        })?;
    let blob = call(&file, "getFile", &[])
        .await
        .map_err(js_err("opening the device's database"))?;
    let buffer = call(&blob, "arrayBuffer", &[])
        .await
        .map_err(js_err("reading the device's database"))?;
    let repo = ClientRepo::from_bytes(&Uint8Array::new(&buffer).to_vec())?;
    Ok(repo.saving_with(Box::new(move |bytes| {
        let file = file.clone();
        Box::pin(async move { write(&file, bytes).await })
    })))
}

async fn root() -> Result<JsValue> {
    let global = js_sys::global();
    let secure = get(&global, "isSecureContext").is_ok_and(|v| v.is_truthy());
    if !secure {
        return Err(RepoError::Unavailable(
            "the device's database needs https (open decision 9)".into(),
        ));
    }
    let storage = get(&global, "navigator")
        .and_then(|navigator| get(&navigator, "storage"))
        .ok()
        .filter(|storage| get(storage, "getDirectory").is_ok_and(|f| f.is_function()))
        .ok_or_else(|| RepoError::Unavailable("this browser has no OPFS".into()))?;
    call(&storage, "getDirectory", &[])
        .await
        .map_err(js_err("opening OPFS"))
}

/// Swap the whole file in. `createWritable` writes to a scratch copy that
/// replaces the file only on `close`, so a write cut short leaves the last
/// complete one.
async fn write(file: &JsValue, bytes: Vec<u8>) -> Result<()> {
    let out = call(file, "createWritable", &[])
        .await
        .map_err(js_err("this browser cannot write the device's database"))?;
    let data: JsValue = Uint8Array::from(bytes.as_slice()).into();
    let written = match call(&out, "write", &[data]).await {
        Ok(_) => call(&out, "close", &[]).await,
        Err(e) => Err(e),
    };
    if let Err(e) = written {
        let _ = call(&out, "abort", &[]).await;
        return Err(js_err("saving the device's database")(e));
    }
    Ok(())
}

fn get(target: &JsValue, key: &str) -> std::result::Result<JsValue, JsValue> {
    Reflect::get(target, &JsValue::from_str(key))
}

/// `target.method(...args)`, awaited if it returns a promise.
async fn call(
    target: &JsValue,
    method: &str,
    args: &[JsValue],
) -> std::result::Result<JsValue, JsValue> {
    let function: Function = get(target, method)?.dyn_into()?;
    let value = function.apply(target, &args.iter().collect::<Array>())?;
    match value.dyn_into::<Promise>() {
        Ok(promise) => JsFuture::from(promise).await,
        Err(value) => Ok(value),
    }
}

fn js_err(what: &'static str) -> impl Fn(JsValue) -> RepoError {
    move |e| {
        let why = e
            .dyn_ref::<js_sys::Error>()
            .map(|e| String::from(e.message()))
            .or_else(|| e.as_string())
            .unwrap_or_else(|| format!("{e:?}"));
        RepoError::Unavailable(format!("{what}: {why}"))
    }
}
