use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use crate::types::*;
use crate::state::*;
use crate::utils::*;

static AUDIO_CAPTURE_REFUSED: AtomicBool = AtomicBool::new(false);

fn consent_value(subkey: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let key: Vec<u16> = subkey.encode_utf16().chain(Some(0)).collect();
    let name: Vec<u16> = "Value".encode_utf16().chain(Some(0)).collect();
    let mut buf = [0u16; 32];
    let mut len = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len))
    };
    if status != ERROR_SUCCESS { return None; }
    let n = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n]))
}

fn microphone_blocked() -> bool {
    const BASE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
    if consent_value(BASE).as_deref() == Some("Deny") { return true; }
    std::env::current_exe().ok().is_some_and(|exe| {
        let app_key = exe.to_string_lossy().replace('\\', "#");
        consent_value(&format!("{BASE}\\NonPackaged\\{app_key}")).as_deref() == Some("Deny")
    })
}

fn audio_visualization_wanted(app: &AppHandle) -> bool {
    let setting = crate::utils::get_setting_str(app, "fern-media-visualizer-enabled")
        .or_else(|| crate::utils::get_setting_str(app, "fern-visualizer-enabled"));
    ANY_MEDIA_PLAYING.load(Ordering::Relaxed)
        && setting.as_deref() != Some("false")
        && !AUDIO_CAPTURE_REFUSED.load(Ordering::Relaxed)
        && !microphone_blocked()
}

unsafe fn is_ieee_float_format(format_ptr: *const windows::Win32::Media::Audio::WAVEFORMATEX) -> bool {
    const WAVE_FORMAT_IEEE_FLOAT_TAG: u16 = 0x0003;
    const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xFFFE;
    let wf = &*format_ptr;
    if wf.wFormatTag == WAVE_FORMAT_IEEE_FLOAT_TAG {
        return true;
    }
    if wf.wFormatTag == WAVE_FORMAT_EXTENSIBLE_TAG && wf.cbSize >= 22 {
        const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: windows::core::GUID =
            windows::core::GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
        let ext_ptr = format_ptr as *const windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE;
        let subformat = std::ptr::read_unaligned(std::ptr::addr_of!((*ext_ptr).SubFormat));
        return subformat == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
    }
    false
}

fn decode_audio_frame(frame_bytes: &[u8], channels: usize, bytes_per_sample: usize, is_float: bool) -> f32 {
    if channels == 0 || bytes_per_sample == 0 || frame_bytes.len() < channels * bytes_per_sample {
        return 0.0;
    }
    let mut sum = 0.0f64;
    for ch in 0..channels {
        let start = ch * bytes_per_sample;
        let s = &frame_bytes[start..start + bytes_per_sample];
        let val = if is_float && bytes_per_sample == 4 {
            let f = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
            if f.is_finite() { (f as f64).clamp(-1.0, 1.0) } else { 0.0 }
        } else if !is_float && bytes_per_sample == 2 {
            (i16::from_le_bytes([s[0], s[1]]) as f64) / 32768.0
        } else if !is_float && bytes_per_sample == 4 {
            (i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64) / 2147483648.0
        } else {
            0.0
        };
        sum += val;
    }
    (sum / channels as f64) as f32
}

pub fn setup_audio_visualization(app_handle: AppHandle) {
    std::thread::spawn(move || {
        use windows::Win32::Media::Audio::{
            IMMDeviceEnumerator, IMMDevice, IAudioClient, IAudioCaptureClient,
            eRender, eConsole, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
        };
        use windows::Win32::System::Com::{
            CoInitializeEx, CoTaskMemFree,
            COINIT_APARTMENTTHREADED, CoCreateInstance, CLSCTX_ALL
        };
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

            const NUM_BANDS: usize = 5;
            let mut max_band_energies = [0.01f32; NUM_BANDS];
            let mut prev_values = [0.1f32; NUM_BANDS];

            let mut failures = 0u32;
            loop {
                if !audio_visualization_wanted(&app_handle) {
                    std::thread::sleep(crate::memory_saver::poll_interval(std::time::Duration::from_millis(1000)));
                    continue;
                }
                let result: Result<(), String> = (|| {
                    let enumerator: IMMDeviceEnumerator =
                        CoCreateInstance(&windows::Win32::Media::Audio::MMDeviceEnumerator, None, CLSCTX_ALL)
                            .map_err(|e| format!("CoCreateInstance failed: {:?}", e))?;

                    let device: IMMDevice = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)
                        .map_err(|e| format!("GetDefaultAudioEndpoint failed: {:?}", e))?;

                    let current_id_str = if let Ok(id) = device.GetId() {
                        let id_str = windows::core::PCWSTR::from_raw(id.0).to_string().unwrap_or_default();
                        CoTaskMemFree(Some(id.0 as *const _));
                        id_str
                    } else { String::new() };

                    let audio_client: IAudioClient = device.Activate(CLSCTX_ALL, None)
                        .map_err(|e| format!("Activate failed: {:?}", e))?;

                    let format_ptr = audio_client.GetMixFormat()
                        .map_err(|e| format!("GetMixFormat failed: {:?}", e))?;
                    struct MixFormatGuard(*mut windows::Win32::Media::Audio::WAVEFORMATEX);
                    impl Drop for MixFormatGuard {
                        fn drop(&mut self) {
                            unsafe { CoTaskMemFree(Some(self.0 as *const _)); }
                        }
                    }
                    let _format_guard = MixFormatGuard(format_ptr);

                    let channels = (*format_ptr).nChannels as usize;
                    let bits_per_sample = (*format_ptr).wBitsPerSample;
                    let bytes_per_sample = (bits_per_sample / 8) as usize;
                    let is_float = is_ieee_float_format(format_ptr);

                    if channels == 0
                        || (is_float && bits_per_sample != 32)
                        || (!is_float && bits_per_sample != 16 && bits_per_sample != 32)
                    {
                        return Err(format!(
                            "Invalid audio format: channels={}, bits={}, float={}",
                            channels, bits_per_sample, is_float
                        ));
                    }

                    let buffer_duration = 10_000_000i64;
                    audio_client.Initialize(
                        AUDCLNT_SHAREMODE_SHARED,
                        AUDCLNT_STREAMFLAGS_LOOPBACK,
                        buffer_duration,
                        0,
                        format_ptr,
                        Some(std::ptr::null()),
                    ).map_err(|e| format!("Initialize failed: {:?}", e))?;

                    drop(_format_guard);

                    let capture_client: IAudioCaptureClient = audio_client.GetService()
                        .map_err(|e| format!("GetService failed: {:?}", e))?;

                    audio_client.Start()
                        .map_err(|e| format!("Start failed: {:?}", e))?;

                    const FFT_SIZE: usize = 512;
                    let mut fft_buffer = vec![0.0f32; FFT_SIZE];
                    let mut fft_input: Vec<rustfft::num_complex::Complex<f32>> = vec![rustfft::num_complex::Complex::new(0.0, 0.0); FFT_SIZE];
                    let mut buffer_pos = 0;

                    let mut planner = rustfft::FftPlanner::<f32>::new();
                    let fft = planner.plan_fft_forward(FFT_SIZE);

                    let mut last_device_check = std::time::Instant::now();
                    let mut idle_since = std::time::Instant::now();
                    loop {
                        let idle = !ANY_MEDIA_PLAYING.load(Ordering::Relaxed);
                        if !idle {
                            idle_since = std::time::Instant::now();
                        } else if idle_since.elapsed() > std::time::Duration::from_secs(15) {
                            return Ok(());
                        }
                        std::thread::sleep(std::time::Duration::from_millis(if idle { 200 } else { 32 }));

                        if last_device_check.elapsed().as_secs() >= 2 {
                            last_device_check = std::time::Instant::now();
                            if let Ok(new_device) = enumerator.GetDefaultAudioEndpoint(eRender, eConsole) {
                                if let Ok(new_id) = new_device.GetId() {
                                    let new_str = windows::core::PCWSTR::from_raw(new_id.0).to_string().unwrap_or_default();
                                    CoTaskMemFree(Some(new_id.0 as *const _));
                                    if current_id_str != new_str {
                                        return Err("Default device changed".into());
                                    }
                                }
                            }
                        }
                        
                        // Skip all heavy processing if nothing is playing
                        if !ANY_MEDIA_PLAYING.load(Ordering::Relaxed) {
                            // Still gotta clear the buffer to avoid lag when it starts
                            loop {
                                let len = match capture_client.GetNextPacketSize() {
                                    Ok(l) => l,
                                    Err(_) => {
                                        return Err("Device invalidated".into());
                                    }
                                };
                                if len == 0 { break; }
                                let mut data_ptr: *mut u8 = std::ptr::null_mut();
                                let mut num_frames = 0u32;
                                let mut flags = 0u32;
                                if capture_client.GetBuffer(&mut data_ptr, &mut num_frames, &mut flags, None, None).is_err() {
                                    return Err("Device invalidated".into());
                                }
                                let _ = capture_client.ReleaseBuffer(num_frames);
                            }
                            continue;
                        }

                        let mut device_invalidated = false;
                        loop {
                            let packet_length = match capture_client.GetNextPacketSize() {
                                Ok(len) => len,
                                Err(_) => { 
                                    device_invalidated = true; 
                                    break; 
                                },
                            };
                            if packet_length == 0 { break; }
                            let mut data_ptr: *mut u8 = std::ptr::null_mut();
                            let mut num_frames = 0u32;
                            let mut flags = 0u32;

                            if capture_client.GetBuffer(&mut data_ptr, &mut num_frames, &mut flags, None, None).is_err() {
                                device_invalidated = true;
                                break;
                            }

                            if num_frames > 0 && bytes_per_sample > 0 && channels > 0 {
                                let is_silent = (flags & (windows::Win32::Media::Audio::AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)) != 0
                                    || data_ptr.is_null();
                                let stride = channels * bytes_per_sample;
                                let raw_slice = if is_silent {
                                    &[]
                                } else {
                                    std::slice::from_raw_parts(data_ptr, num_frames as usize * stride)
                                };
                                for frame in 0..num_frames as usize {
                                    let normalized = if is_silent {
                                        0.0
                                    } else {
                                        let frame_bytes = &raw_slice[frame * stride..(frame + 1) * stride];
                                        decode_audio_frame(frame_bytes, channels, bytes_per_sample, is_float)
                                    };
                                    if buffer_pos < FFT_SIZE {
                                        let window = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * buffer_pos as f32 / FFT_SIZE as f32).cos());
                                        fft_buffer[buffer_pos] = normalized * window;
                                        buffer_pos += 1;
                                    }
                                    if buffer_pos >= FFT_SIZE {
                                        // Run FFT on the windowed buffer
                                        for (i, &sample) in fft_buffer.iter().enumerate() {
                                            fft_input[i] = rustfft::num_complex::Complex::new(sample, 0.0);
                                        }
                                        fft.process(&mut fft_input);

                                        let band_ranges = [(1, 2), (2, 6), (6, 18), (18, 60), (60, 200)];
                                        let mut output = [0.0f32; NUM_BANDS];
                                        for (band_idx, &(bin_start, bin_end)) in band_ranges.iter().enumerate() {
                                            let mut total_mag = 0.0f32;
                                            let mut count = 0u32;
                                            for bin in bin_start..bin_end {
                                                if bin >= FFT_SIZE / 2 { break; }
                                                let mag = fft_input[bin].norm();
                                                total_mag += mag;
                                                count += 1;
                                            }
                                            let avg_mag = total_mag / count.max(1) as f32;
                                            let mut scaled_mag = avg_mag;
                                            let weighting = [1.2, 1.2, 1.5, 2.8, 5.0];
                                            scaled_mag *= weighting[band_idx];
                                            if scaled_mag > max_band_energies[band_idx] {
                                                max_band_energies[band_idx] = scaled_mag;
                                            } else {
                                                max_band_energies[band_idx] *= 0.99;
                                            }
                                            let target = (scaled_mag / max_band_energies[band_idx].max(0.12)).min(1.0).powf(0.75);
                                            let is_rising = target > prev_values[band_idx];
                                            let smooth_factor = if is_rising { 0.10 } else { 0.20 };
                                            output[band_idx] = prev_values[band_idx] * smooth_factor + target * (1.0 - smooth_factor);
                                            output[band_idx] = output[band_idx].clamp(0.18, 1.0);
                                            prev_values[band_idx] = output[band_idx];
                                        }
                                        let _ = app_handle.emit("audio-visualization", AudioVisualizationData { frequencies: output.to_vec() });
                                        buffer_pos = 0;
                                    }
                                }
                                let _ = capture_client.ReleaseBuffer(num_frames);
                            }
                        }
                        
                        if device_invalidated {
                            return Err("Device invalidated".into());
                        }
                    }
                })();

                if let Err(e) = &result {
                    if e.contains("0x80070005") {
                        AUDIO_CAPTURE_REFUSED.store(true, Ordering::Relaxed);
                        crate::diagnostics::log("audio visualizer: Windows refused the capture stream, not retrying this session");
                    }
                }
                failures = match &result {
                    Err(e) if e.contains(" failed") => (failures + 1).min(8),
                    _ => 0,
                };
                let wait_ms = 2500u64 << failures.min(7);
                std::thread::sleep(std::time::Duration::from_millis(wait_ms.min(300_000)));
            }
            // if com_initialized { CoUninitialize(); }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_frame_decoder_handles_ieee_float_and_pcm() {
        use super::decode_audio_frame;
        // Stereo 32-bit IEEE float (+0.5, -0.25) -> avg 0.125
        let mut float_frame = Vec::new();
        float_frame.extend_from_slice(&0.5f32.to_le_bytes());
        float_frame.extend_from_slice(&(-0.25f32).to_le_bytes());
        let val = decode_audio_frame(&float_frame, 2, 4, true);
        assert!((val - 0.125).abs() < 1e-6);

        // NaN float sample should clamp cleanly to 0.0
        let nan_frame = f32::NAN.to_le_bytes();
        assert_eq!(decode_audio_frame(&nan_frame, 1, 4, true), 0.0);

        // Mono 16-bit PCM half-scale negative (-16384 -> -0.5)
        let pcm16 = (-16384i16).to_le_bytes();
        assert!((decode_audio_frame(&pcm16, 1, 2, false) - (-0.5)).abs() < 1e-6);

        // Mono 32-bit PCM half-scale positive
        let pcm32 = (i32::MAX / 2).to_le_bytes();
        assert!((decode_audio_frame(&pcm32, 1, 4, false) - 0.5).abs() < 1e-4);
    }
}
