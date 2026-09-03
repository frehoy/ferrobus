//! Persisting prebuilt artifacts to disk and loading them back.
//!
//! Building a [`TransitModel`] means parsing an OSM extract and one or more GTFS
//! feeds, then computing transfers between every pair of nearby stops. For a
//! city-sized dataset that is seconds to minutes of work, and it produces the
//! exact same model every time. The same goes for an [`IsochroneIndex`], which
//! runs a Dijkstra search per grid cell.
//!
//! This module lets that work happen once. The resulting file is a compact
//! binary snapshot ([postcard](https://docs.rs/postcard) encoded) that loads
//! back into memory without touching the original OSM or GTFS data.
//!
//! ```no_run
//! use ferrobus_core::{TransitModelConfig, create_transit_model, persist};
//!
//! let config = TransitModelConfig::default();
//! let model = create_transit_model(&config)?;
//! persist::save_transit_model(&model, "model.ferrobus")?;
//!
//! // Later, in another process:
//! let model = persist::load_transit_model("model.ferrobus")?;
//! # Ok::<(), ferrobus_core::Error>(())
//! ```
//!
//! # Compatibility
//!
//! Files are **not** a stable interchange format. The header records the format
//! version and the ferrobus version that wrote the file; loading refuses
//! anything it does not recognise, so a stale cache produces a clear error
//! rather than a corrupt model. Rebuild from source data after upgrading.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use log::info;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::algo::IsochroneIndex;
use crate::loading::{TransitModelConfig, create_transit_model};
use crate::{Error, TransitModel};

/// Magic bytes every ferrobus artifact file starts with.
const MAGIC: &[u8; 8] = b"FERROBUS";

/// On-disk layout version.
///
/// Bump this whenever a persisted structure changes shape, so that older files
/// are rejected instead of being silently misread.
const FORMAT_VERSION: u16 = 2;

/// Scratch for streaming decode: enough for the longest `String`, but a borrowed field would consume it cumulatively.
const SCRATCH_LEN: usize = 1 << 20;

/// Which kind of artifact a file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactKind {
    TransitModel = 1,
    IsochroneIndex = 2,
}

impl ArtifactKind {
    fn name(self) -> &'static str {
        match self {
            ArtifactKind::TransitModel => "transit model",
            ArtifactKind::IsochroneIndex => "isochrone index",
        }
    }

    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(ArtifactKind::TransitModel),
            2 => Some(ArtifactKind::IsochroneIndex),
            _ => None,
        }
    }
}

fn encoding_error(err: &postcard::Error) -> Error {
    Error::Serialization(err.to_string())
}

/// Writes the file header followed by the postcard-encoded payload.
fn write_artifact<T: Serialize>(value: &T, kind: ArtifactKind, path: &Path) -> Result<(), Error> {
    let crate_version = env!("CARGO_PKG_VERSION").as_bytes();
    let version_len = u32::try_from(crate_version.len())
        .map_err(|_| Error::Serialization("crate version string is too long".to_string()))?;

    let file = File::create(path)?;
    let mut writer = BufWriter::with_capacity(1 << 20, file);

    writer.write_all(MAGIC)?;
    writer.write_all(&FORMAT_VERSION.to_le_bytes())?;
    writer.write_all(&[kind as u8])?;
    writer.write_all(&version_len.to_le_bytes())?;
    writer.write_all(crate_version)?;

    let mut writer = postcard::to_io(value, writer).map_err(|e| encoding_error(&e))?;
    writer.flush()?;

    Ok(())
}

/// Validates the file header and decodes the payload.
fn read_artifact<T: DeserializeOwned>(kind: ArtifactKind, path: &Path) -> Result<T, Error> {
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(1 << 20, file);

    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic).map_err(|_| {
        Error::IncompatibleFormat(format!(
            "{} is not a ferrobus artifact file",
            path.display()
        ))
    })?;
    if &magic != MAGIC {
        return Err(Error::IncompatibleFormat(format!(
            "{} is not a ferrobus artifact file",
            path.display()
        )));
    }

    let mut version_bytes = [0u8; 2];
    reader.read_exact(&mut version_bytes)?;
    let format_version = u16::from_le_bytes(version_bytes);
    if format_version != FORMAT_VERSION {
        return Err(Error::IncompatibleFormat(format!(
            "{} uses format version {format_version}, this build reads version {FORMAT_VERSION}; \
             rebuild the artifact from the source data",
            path.display()
        )));
    }

    let mut kind_byte = [0u8; 1];
    reader.read_exact(&mut kind_byte)?;
    let file_kind = ArtifactKind::from_byte(kind_byte[0]).ok_or_else(|| {
        Error::IncompatibleFormat(format!(
            "{} holds an unknown artifact kind ({})",
            path.display(),
            kind_byte[0]
        ))
    })?;
    if file_kind != kind {
        return Err(Error::IncompatibleFormat(format!(
            "{} holds a {}, expected a {}",
            path.display(),
            file_kind.name(),
            kind.name()
        )));
    }

    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes)?;
    let version_len = u32::from_le_bytes(len_bytes) as usize;
    let mut crate_version = vec![0u8; version_len];
    reader.read_exact(&mut crate_version)?;
    let crate_version = String::from_utf8_lossy(&crate_version).into_owned();
    if crate_version != env!("CARGO_PKG_VERSION") {
        return Err(Error::IncompatibleFormat(format!(
            "{} was written by ferrobus {crate_version}, this build is {}; \
             rebuild the artifact from the source data",
            path.display(),
            env!("CARGO_PKG_VERSION")
        )));
    }

    // Only has to fit the longest string: postcard copies owned values out.
    let mut scratch = vec![0u8; SCRATCH_LEN];
    let (value, _) =
        postcard::from_io((reader, scratch.as_mut_slice())).map_err(|e| encoding_error(&e))?;

    Ok(value)
}

/// Writes a prebuilt transit model to `path`.
///
/// The file is self-contained: loading it needs neither the OSM extract nor the
/// GTFS feeds it was built from.
///
/// # Errors
///
/// Returns an error if the file cannot be written or the model cannot be encoded.
pub fn save_transit_model<P: AsRef<Path>>(model: &TransitModel, path: P) -> Result<(), Error> {
    let path = path.as_ref();
    info!("Saving transit model to {}", path.display());
    write_artifact(model, ArtifactKind::TransitModel, path)
}

/// Loads a transit model previously written by [`save_transit_model`].
///
/// The loaded model is audited for structural consistency before it is returned,
/// so a truncated or corrupted file fails here rather than during routing.
///
/// # Errors
///
/// Returns an error if the file is missing, was written by a different ferrobus
/// version, holds a different kind of artifact, or fails the structural audit.
pub fn load_transit_model<P: AsRef<Path>>(path: P) -> Result<TransitModel, Error> {
    let path = path.as_ref();
    info!("Loading transit model from {}", path.display());

    let model: TransitModel = read_artifact(ArtifactKind::TransitModel, path)?;
    crate::model::audit_transit_model(&model)?;

    info!(
        "Loaded transit model with {} stops and {} routes",
        model.stop_count(),
        model.route_count()
    );
    Ok(model)
}

/// Writes a prebuilt isochrone index to `path`.
///
/// # Errors
///
/// Returns an error if the file cannot be written or the index cannot be encoded.
pub fn save_isochrone_index<P: AsRef<Path>>(index: &IsochroneIndex, path: P) -> Result<(), Error> {
    let path = path.as_ref();
    info!("Saving isochrone index to {}", path.display());
    write_artifact(index, ArtifactKind::IsochroneIndex, path)
}

/// Loads an isochrone index previously written by [`save_isochrone_index`].
///
/// The index stores street network node indices, so it is only valid together
/// with the transit model it was built against. Persist both, and reload both.
///
/// # Errors
///
/// Returns an error if the file is missing, was written by a different ferrobus
/// version, or holds a different kind of artifact.
pub fn load_isochrone_index<P: AsRef<Path>>(path: P) -> Result<IsochroneIndex, Error> {
    let path = path.as_ref();
    info!("Loading isochrone index from {}", path.display());
    read_artifact(ArtifactKind::IsochroneIndex, path)
}

/// Loads the model cached at `cache_path`, building and caching it if absent.
///
/// This is the build-once workflow in a single call: the first run pays for OSM
/// and GTFS processing, every later run reads the snapshot instead.
///
/// A cache written by a different ferrobus version is *not* silently reused --
/// it is rebuilt from `config` and overwritten. A cache that exists but cannot
/// be read for any other reason (I/O failure, wrong artifact kind) is an error,
/// so genuine problems are not papered over.
///
/// Note that the cache is keyed only by path: ferrobus cannot tell whether
/// `config` still describes the cached model. Use a distinct path per
/// configuration, or delete the file when the inputs change.
///
/// # Errors
///
/// Returns an error if the cache exists but cannot be read, or if building the
/// model from `config` fails.
pub fn load_or_create_transit_model<P: AsRef<Path>>(
    config: &TransitModelConfig,
    cache_path: P,
) -> Result<TransitModel, Error> {
    let cache_path = cache_path.as_ref();

    if cache_path.exists() {
        match load_transit_model(cache_path) {
            Ok(model) => return Ok(model),
            Err(Error::IncompatibleFormat(reason)) => {
                log::warn!("Ignoring cached transit model: {reason}");
            }
            Err(err) => return Err(err),
        }
    }

    let model = create_transit_model(config)?;
    save_transit_model(&model, cache_path)?;
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> std::path::PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();
        std::env::temp_dir().join(format!("ferrobus_persist_{name}_{}_{ts}", process::id()))
    }

    #[test]
    fn rejects_file_without_magic() {
        let path = temp_path("bad_magic");
        std::fs::write(&path, b"not a ferrobus file at all").expect("temp file should be written");

        let result = load_transit_model(&path);
        std::fs::remove_file(&path).ok();

        assert!(matches!(result, Err(Error::IncompatibleFormat(_))));
    }

    #[test]
    fn rejects_wrong_artifact_kind() {
        let path = temp_path("wrong_kind");
        let index = IsochroneIndex::empty_for_tests();
        save_isochrone_index(&index, &path).expect("index should be saved");

        let result = load_transit_model(&path);
        std::fs::remove_file(&path).ok();

        assert!(matches!(result, Err(Error::IncompatibleFormat(_))));
    }

    #[test]
    fn isochrone_index_round_trips() {
        let path = temp_path("index_round_trip");
        let index = IsochroneIndex::empty_for_tests();
        save_isochrone_index(&index, &path).expect("index should be saved");

        let loaded = load_isochrone_index(&path).expect("index should be loaded");
        std::fs::remove_file(&path).ok();

        assert_eq!(loaded.len(), index.len());
        assert_eq!(loaded.resolution(), index.resolution());
    }
}
