//! libm symbols missing from the TV's glibc.
//!
//! Recent Rust lowers `f32::max`/`f32::min` to the C23 `fmaximum_numf`/`fminimum_numf`,
//! which glibc only ships since 2.35. webOS 5/6 TVs run glibc 2.28, so without these
//! definitions the binary either fails to link or fails to load on the TV.
//! The bodies avoid `f32::max`/`f32::min` so they cannot recurse into themselves.

#[cfg(all(target_arch = "arm", target_env = "gnu"))]
#[no_mangle]
pub extern "C" fn fmaximum_numf(x: f32, y: f32) -> f32 {
    if x.is_nan() {
        return y;
    }
    if y.is_nan() {
        return x;
    }
    if x > y || (x == y && y.is_sign_negative()) {
        x
    } else {
        y
    }
}

#[cfg(all(target_arch = "arm", target_env = "gnu"))]
#[no_mangle]
pub extern "C" fn fminimum_numf(x: f32, y: f32) -> f32 {
    if x.is_nan() {
        return y;
    }
    if y.is_nan() {
        return x;
    }
    if x < y || (x == y && x.is_sign_negative()) {
        x
    } else {
        y
    }
}
