//! Govee RGBIC strips over the LAN API "razer" (DreamView) segment mode on UDP port 4003.
//!
//! Enabling razer mode hands the strip to us; disabling it returns the strip to whatever scene
//! it was showing before, so releasing is always clean. The strip drops out of razer mode after a
//! few seconds without an enable command, so the enable packet is repeated while streaming.

use anyhow::{Context, Result};
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};
use tracing::{error, info};

use crate::color::{ActiveRect, ColorProcessor, RgbColor};
use crate::config::GoveeConfig;

pub const GOVEE_CONTROL_PORT: u16 = 4003;
const RAZER_KEEPALIVE: Duration = Duration::from_secs(5);

/// Frames a razer payload: appends the XOR checksum and wraps it in the LAN API JSON envelope.
pub fn razer_message(payload: &[u8]) -> String {
    let checksum = payload.iter().fold(0u8, |acc, byte| acc ^ byte);
    let mut framed = payload.to_vec();
    framed.push(checksum);
    format!(
        r#"{{"msg":{{"cmd":"razer","data":{{"pt":"{}"}}}}}}"#,
        base64_encode(&framed)
    )
}

pub fn razer_enable_payload(enabled: bool) -> [u8; 5] {
    [0xBB, 0x00, 0x01, 0xB1, u8::from(enabled)]
}

/// Segment colour payload: `BB 00 <len> B0 01 <count> (r g b)*`.
pub fn razer_colors_payload(colors: &[RgbColor]) -> Vec<u8> {
    let count = colors.len().min(u8::MAX as usize);
    let mut payload = Vec::with_capacity(6 + count * 3);
    payload.extend_from_slice(&[0xBB, 0x00, (count * 3 + 2) as u8, 0xB0, 0x01, count as u8]);
    for c in colors.iter().take(count) {
        payload.extend_from_slice(&[c.r, c.g, c.b]);
    }
    payload
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub struct GoveeRazerStreamer {
    socket: UdpSocket,
    target_addr: SocketAddr,
    enabled: bool,
    last_enable: Instant,
}

impl GoveeRazerStreamer {
    pub fn new(ip: &str) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .context("Failed to bind UDP socket for Govee streaming")?;
        socket.set_nonblocking(true)?;
        let target_addr: SocketAddr = format!("{ip}:{GOVEE_CONTROL_PORT}")
            .parse()
            .with_context(|| format!("Invalid Govee address: {ip}"))?;
        socket.connect(target_addr)?;
        Ok(Self {
            socket,
            target_addr,
            enabled: false,
            last_enable: Instant::now(),
        })
    }

    fn send_payload(&self, payload: &[u8]) -> Result<()> {
        match self.socket.send(razer_message(payload).as_bytes()) {
            Ok(_) => Ok(()),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::Interrupted =>
            {
                Ok(())
            }
            Err(e) => {
                error!(
                    "Failed to send Govee razer packet to {}: {}",
                    self.target_addr, e
                );
                Err(e.into())
            }
        }
    }

    /// Takes over the strip (`true`) or hands it back to its own scene (`false`).
    pub fn set_enabled(&mut self, enabled: bool) -> Result<()> {
        if self.enabled == enabled {
            return Ok(());
        }
        self.send_payload(&razer_enable_payload(enabled))?;
        self.enabled = enabled;
        self.last_enable = Instant::now();
        info!(
            "Govee razer mode {} ({})",
            if enabled { "on" } else { "off" },
            self.target_addr
        );
        Ok(())
    }

    pub fn send_frame(&mut self, colors: &[RgbColor]) -> Result<()> {
        if !self.enabled {
            self.set_enabled(true)?;
        } else if self.last_enable.elapsed() >= RAZER_KEEPALIVE {
            self.send_payload(&razer_enable_payload(true))?;
            self.last_enable = Instant::now();
        }
        self.send_payload(&razer_colors_payload(colors))
    }
}

impl Drop for GoveeRazerStreamer {
    fn drop(&mut self) {
        let _ = self.set_enabled(false);
    }
}

/// Samples one horizontal band of the picture into equal-width segments for a Govee strip.
pub struct GoveeBandSampler {
    segments: usize,
    reverse: bool,
    band_start: f32,
    band_end: f32,
    smoothed_colors: Vec<RgbColor>,
    active_rect: Option<ActiveRect>,
    processor: ColorProcessor,
    brightness_multiplier: f32,
}

impl GoveeBandSampler {
    pub fn new(
        config: &GoveeConfig,
        hdr_tone_mapping: bool,
        saturation_boost: f32,
        noise_gate_threshold: f32,
        brightness_multiplier: f32,
    ) -> Self {
        let segments = usize::from(config.segments.clamp(1, 84));
        let band_start = config.band_start.clamp(0.0, 1.0);
        let band_end = config
            .band_end
            .clamp(0.0, 1.0)
            .max(band_start + 0.02)
            .min(1.0);
        Self {
            segments,
            reverse: config.reverse,
            band_start,
            band_end,
            smoothed_colors: vec![RgbColor::new(0, 0, 0); segments],
            active_rect: None,
            processor: ColorProcessor::new(
                0.35,
                hdr_tone_mapping,
                saturation_boost,
                noise_gate_threshold,
            ),
            brightness_multiplier,
        }
    }

    pub fn processor_mut(&mut self) -> &mut ColorProcessor {
        &mut self.processor
    }

    pub fn set_brightness_multiplier(&mut self, mult: f32) {
        self.brightness_multiplier = mult.clamp(0.1, 4.0);
    }

    pub fn set_active_rect(&mut self, rect: ActiveRect) {
        self.active_rect = Some(rect);
    }

    pub fn sample_frame(
        &mut self,
        frame_data: &[u8],
        width: u32,
        height: u32,
        is_bgra: bool,
        is_scene_cut: bool,
    ) -> Vec<RgbColor> {
        if width == 0 || height == 0 {
            return vec![RgbColor::new(0, 0, 0); self.segments];
        }
        let rect = self.active_rect.unwrap_or_default();
        let x_range = (rect.x_max - rect.x_min).max(0.1);
        let y_range = (rect.y_max - rect.y_min).max(0.1);
        let y_min = rect.y_min + self.band_start * y_range;
        let y_max = rect.y_min + self.band_end * y_range;
        let y_start = ((y_min * height as f32) as u32).min(height - 1);
        let y_end = ((y_max * height as f32) as u32)
            .min(height)
            .max(y_start + 1);

        let mut result = Vec::with_capacity(self.segments);
        for i in 0..self.segments {
            let x_min = rect.x_min + (i as f32 / self.segments as f32) * x_range;
            let x_max = rect.x_min + ((i + 1) as f32 / self.segments as f32) * x_range;
            let x_start = ((x_min * width as f32) as u32).min(width - 1);
            let x_end = ((x_max * width as f32) as u32).min(width).max(x_start + 1);

            let (mut wr, mut wg, mut wb, mut total) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            let mut peak = RgbColor::new(0, 0, 0);
            let mut max_luma = 0.0f32;
            for y in y_start..y_end {
                let row = (y * width * 4) as usize;
                for x in x_start..x_end {
                    let idx = row + (x * 4) as usize;
                    if idx + 2 >= frame_data.len() {
                        continue;
                    }
                    let (r, g, b) = if is_bgra {
                        (frame_data[idx + 2], frame_data[idx + 1], frame_data[idx])
                    } else {
                        (frame_data[idx], frame_data[idx + 1], frame_data[idx + 2])
                    };
                    let pix = RgbColor::new(r, g, b);
                    let luma = pix.luminance();
                    if luma > max_luma {
                        max_luma = luma;
                        peak = pix;
                    }
                    let weight = self.processor.sample_weight(pix.saturation());
                    wr += r as f32 * weight;
                    wg += g as f32 * weight;
                    wb += b as f32 * weight;
                    total += weight;
                }
            }
            let mean = if total > 0.0 {
                RgbColor::new(
                    (wr / total).clamp(0.0, 255.0) as u8,
                    (wg / total).clamp(0.0, 255.0) as u8,
                    (wb / total).clamp(0.0, 255.0) as u8,
                )
            } else {
                RgbColor::new(0, 0, 0)
            };
            let smoothed =
                self.processor
                    .process(mean, peak, max_luma, self.smoothed_colors[i], is_scene_cut);
            self.smoothed_colors[i] = smoothed;
            result.push(smoothed.scale(self.brightness_multiplier));
        }
        if self.reverse {
            result.reverse();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(segments: u8, reverse: bool) -> GoveeConfig {
        GoveeConfig {
            enabled: true,
            ip: "192.0.2.10".into(),
            segments,
            reverse,
            band_start: 0.0,
            band_end: 0.5,
        }
    }

    #[test]
    fn razer_messages_match_the_lan_api_wire_format() {
        // Enable: BB 00 01 B1 01 + xor(0x0A) -> "uwABsQEK"
        assert_eq!(
            razer_message(&razer_enable_payload(true)),
            r#"{"msg":{"cmd":"razer","data":{"pt":"uwABsQEK"}}}"#
        );
        let payload = razer_colors_payload(&[RgbColor::new(255, 0, 0), RgbColor::new(0, 0, 255)]);
        assert_eq!(
            payload,
            vec![0xBB, 0x00, 8, 0xB0, 0x01, 2, 255, 0, 0, 0, 0, 255]
        );
    }

    #[test]
    fn base64_handles_padding() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
    }

    fn split_frame(width: u32, height: u32) -> Vec<u8> {
        // Left half red, right half blue.
        let mut frame = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let i = ((y * width + x) * 4) as usize;
                if x < width / 2 {
                    frame[i] = 255;
                } else {
                    frame[i + 2] = 255;
                }
                frame[i + 3] = 255;
            }
        }
        frame
    }

    #[test]
    fn segments_follow_screen_left_to_right_and_reverse() {
        let frame = split_frame(64, 36);
        let mut sampler = GoveeBandSampler::new(&config(2, false), false, 1.0, 0.0, 1.0);
        sampler.processor_mut().set_max_color_step(255);
        let colors = sampler.sample_frame(&frame, 64, 36, false, true);
        assert_eq!(colors.len(), 2);
        assert!(
            colors[0].r > 200 && colors[0].b < 50,
            "left segment red: {:?}",
            colors[0]
        );
        assert!(
            colors[1].b > 200 && colors[1].r < 50,
            "right segment blue: {:?}",
            colors[1]
        );

        let mut reversed = GoveeBandSampler::new(&config(2, true), false, 1.0, 0.0, 1.0);
        reversed.processor_mut().set_max_color_step(255);
        let colors = reversed.sample_frame(&frame, 64, 36, false, true);
        assert!(colors[0].b > 200 && colors[1].r > 200);
    }
}
