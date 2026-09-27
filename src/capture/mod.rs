pub mod dile_vt;
pub mod protected;
pub mod vtcapture;

use crate::config::CaptureBackend;
use anyhow::{anyhow, Result};
use tracing::{error, info, warn};

pub struct CapturedFrame<'a> {
    pub data: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub is_bgra: bool,
}

pub trait ScreenCapture: Send {
    fn acquire_frame(&mut self) -> Result<CapturedFrame<'_>>;
    #[allow(dead_code)]
    fn resolution(&self) -> (u32, u32);
    fn is_real_hardware(&self) -> bool {
        false
    }
}

pub use dile_vt::{detect_source_fps, DileVtCapture, MockCapture};
pub use vtcapture::VtCapture;

/// Hidden CLI subcommand used to probe libvtcapture in a child process.
pub const VTCAPTURE_PROBE_COMMAND: &str = "probe-vtcapture";

/// `vtCapture_create()` registers on the Luna bus and throws a C++ exception (aborting the whole
/// process) when this executable has no Luna role, e.g. when run from outside the provisioned
/// install path. Probe it once in a child process so a missing role cannot take the daemon down.
fn vtcapture_usable() -> bool {
    static USABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *USABLE.get_or_init(|| {
        if !std::path::Path::new(vtcapture::LIBVTCAPTURE_PATH).exists() {
            return false;
        }
        let status = std::env::current_exe().and_then(|exe| {
            std::process::Command::new(exe)
                .arg(VTCAPTURE_PROBE_COMMAND)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
        });
        match status {
            Ok(status) if status.success() => true,
            Ok(status) => {
                warn!(
                    "libvtcapture aborted in a probe process ({status}); it needs a Luna role for this executable. \
                     Install to /var/home/root/lg-hue-sync and run scripts/provision_luna.sh, or set \
                     \"capture_backend\": \"dile_vt\". Using libdile_vt instead."
                );
                false
            }
            Err(error) => {
                warn!("Could not probe libvtcapture ({error}); using libdile_vt instead.");
                false
            }
        }
    })
}

/// Entry point of the probe child: only creates and releases a libvtcapture driver.
pub fn run_vtcapture_probe() -> i32 {
    match vtcapture::probe_driver() {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

fn busy_error(error: &anyhow::Error) -> bool {
    let text = error.to_string();
    text.contains("error code 11") || text.contains("EBUSY")
}

/// Opens a hardware capture backend, honouring the configured preference.
/// `Auto` tries libvtcapture (webOS 6 / C1) first, then libdile_vt (webOS 3-5, e.g. BX/CX).
pub fn try_hardware_capture(
    backend: CaptureBackend,
    width: u32,
    height: u32,
) -> Result<Box<dyn ScreenCapture>> {
    let mut errors = Vec::new();

    // Probe even when libvtcapture is requested explicitly: without a Luna role it aborts the
    // calling process, and an explicit preference must not turn that into a daemon crash.
    let try_vtcapture = match backend {
        CaptureBackend::Auto | CaptureBackend::Vtcapture => vtcapture_usable(),
        CaptureBackend::DileVt => false,
    };
    if backend == CaptureBackend::Vtcapture && !try_vtcapture {
        errors.push("vtcapture: unusable in this process (see probe warning)".to_string());
    }
    if try_vtcapture {
        match VtCapture::try_new(width, height) {
            Ok(capture) => {
                info!("Initialized VtCapture driver (/usr/lib/libvtcapture.so.1) successfully.");
                return Ok(Box::new(capture));
            }
            Err(e) => {
                if busy_error(&e) {
                    error!(
                        "[-] Capture device is BUSY: /dev/video* hardware scaler is in use by another process (e.g. lg-hue-sync.service, PicCap). Stop it before starting a manual capture session."
                    );
                }
                errors.push(format!("vtcapture: {e}"));
            }
        }
    }

    if matches!(backend, CaptureBackend::Auto | CaptureBackend::DileVt) {
        match DileVtCapture::try_new(width, height, 0) {
            Ok(capture) => {
                info!("Initialized DileVtCapture driver (/usr/lib/libdile_vt.so.0) successfully.");
                return Ok(Box::new(capture));
            }
            Err(e) => {
                if busy_error(&e) {
                    error!("[-] DileVtCapture device is BUSY: /dev/video* is in use by another process.");
                }
                errors.push(format!("dile_vt: {e}"));
            }
        }
    }

    Err(anyhow!(
        "no hardware capture backend available ({})",
        errors.join("; ")
    ))
}

/// Creates a screen capture instance: a hardware backend when one opens, otherwise MockCapture
/// (black frames on a TV, a test pattern on a development host).
pub fn create_capture(backend: CaptureBackend, width: u32, height: u32) -> Box<dyn ScreenCapture> {
    match try_hardware_capture(backend, width, height) {
        Ok(capture) => capture,
        Err(e) => {
            warn!("{e}. Falling back to MockCapture.");
            Box::new(MockCapture::new(width, height))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_capture_fallback_to_mock() {
        let mut capture = create_capture(CaptureBackend::Auto, 160, 90);
        let frame = capture.acquire_frame().expect("Acquire mock frame failed");
        assert_eq!(frame.width, 160);
        assert_eq!(frame.height, 90);
        assert_eq!(frame.data.len(), 160 * 90 * 4);
    }
}
