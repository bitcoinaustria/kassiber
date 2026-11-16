//! The wallet-material scanner's camera: what `qr-scanner` does in the web (open a camera with
//! `getUserMedia`, show it, decode QR codes in the centre of the frame eight times a second),
//! natively.
//!
//! - **macOS**: AVFoundation capture (`AVCaptureSession`, a device input, an
//!   `AVCaptureVideoDataOutput` delivering BGRA frames on a serial queue). Every frame goes to the
//!   dialog's `NativeSurface` without a copy; at most eight a second, the centre square (72 % of
//!   the short side, `calculateScanRegion`) is decoded with Core Image's QR detector
//!   (`CIDetector`), and each decoded text is handed to the dialog. Nothing leaves the machine.
//! - **Permission**: asked only when the dialog opens (the user pressed Scan), as the web's
//!   camera start does. A refusal, a missing camera or a failed start ends in the web's messages.
//! - **Other platforms** and every headless run (snapshots, scripts, tests): no camera, the web's
//!   "Camera not found." state. That also keeps agent tooling from ever prompting for a camera.
//!
//! Frames and texts never reach a log. The bundle declares `NSCameraUsageDescription`, as the
//! Tauri bundle does. Neither app's signing entitlements carry `com.apple.security.device.camera`
//! (`kassiber-release` pins the exact set), so a hardened-runtime release build is refused the
//! camera and ends in the "denied" state until the release tool adds that entitlement.

use std::sync::{Arc, Mutex};

/// qr-scanner's message when no camera can be opened.
pub const NOT_FOUND: &str = "Camera not found.";
/// WebKit's `NotAllowedError` text, as the web shows a refused camera.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // macOS-only path (and its tests)
pub const DENIED: &str = "The request is not allowed by the user agent or the platform in the current context, possibly because the user denied permission.";

/// A camera the user can switch to (`QrScanner.Camera`).
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub label: String,
}

#[derive(Debug, PartialEq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // macOS-only path (and its tests)
pub enum Event {
    /// The camera runs; the cameras there are (`listCameras`).
    Started(Vec<Device>),
    /// A QR code's text.
    Decoded(String),
    /// The start (or a switch) failed.
    Failed(String),
}

/// What the capture side hands the UI thread.
#[derive(Clone, Default)]
pub struct Inbox(Arc<Mutex<Vec<Event>>>);

impl Inbox {
    pub fn push(&self, e: Event) {
        self.0.lock().unwrap().push(e);
        if let Some(w) = crate::native::waker() {
            w();
        }
    }

    pub fn drain(&self) -> Vec<Event> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

/// A running camera; dropping it stops the capture.
pub struct Camera {
    #[cfg(target_os = "macos")]
    _session: Option<mac::Session>,
}

/// Opens `device` (None: the default camera) into `surface`; events arrive in `inbox`.
pub fn start(device: Option<String>, surface: rune::NativeSurface, inbox: Inbox) -> Camera {
    if rune::is_headless() || cfg!(test) {
        inbox.push(Event::Failed(NOT_FOUND.into()));
        return Camera {
            #[cfg(target_os = "macos")]
            _session: None,
        };
    }
    #[cfg(target_os = "macos")]
    {
        Camera {
            _session: mac::Session::start(device, surface, inbox),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (device, surface);
        inbox.push(Event::Failed(NOT_FOUND.into()));
        Camera {}
    }
}

/// `calculateScanRegion`: the centred square, 72 % of the short side.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // macOS-only path (and its tests)
pub fn scan_region(w: f64, h: f64) -> (f64, f64, f64, f64) {
    let size = (w.min(h) * 0.72).round();
    (
        ((w - size) / 2.0).round(),
        ((h - size) / 2.0).round(),
        size,
        size,
    )
}

/// qr-scanner mirrors a user-facing camera's preview (`_getFacingMode`: front, user or face in
/// the label).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // macOS-only path (and its tests)
pub fn mirrored(label: &str) -> bool {
    let l = label.to_lowercase();
    ["front", "user", "face"].iter().any(|w| l.contains(w))
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject};
    use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
    use objc2_foundation::{NSRect, NSString};

    use super::{DENIED, Device, Event, Inbox, NOT_FOUND, mirrored, scan_region};

    // One `#[link]` per framework is how Rust links several; clippy reads the repeated
    // `kind = "framework"` as a duplicate, but removing any of them drops that framework.
    #[allow(clippy::duplicated_attributes)]
    #[link(name = "AVFoundation", kind = "framework")]
    #[link(name = "CoreMedia", kind = "framework")]
    #[link(name = "CoreImage", kind = "framework")]
    #[link(name = "CoreVideo", kind = "framework")]
    unsafe extern "C" {
        static AVMediaTypeVideo: &'static NSString;
        static CIDetectorTypeQRCode: &'static NSString;
        static CIDetectorAccuracy: &'static NSString;
        static CIDetectorAccuracyHigh: &'static NSString;
        static kCVPixelBufferPixelFormatTypeKey: &'static NSString;
        static kCVPixelBufferMetalCompatibilityKey: &'static NSString;
        fn CMSampleBufferGetImageBuffer(sbuf: *const c_void) -> *mut c_void;
    }

    /// `maxScansPerSecond: 8`.
    const SCAN_EVERY: Duration = Duration::from_millis(125);
    const BGRA: u32 = 0x4247_5241;
    const NOT_DETERMINED: isize = 0;
    const AUTHORIZED: isize = 3;

    /// Objective-C objects used from the capture queue and the starting thread only.
    struct Objc(Retained<AnyObject>);
    // SAFETY: AVCaptureSession and CIDetector are used from one thread at a time here (the start
    // thread, then the serial capture queue); Retained is only moved, never shared mutably.
    unsafe impl Send for Objc {}

    pub struct Shared {
        surface: rune::NativeSurface,
        inbox: Inbox,
        detector: Mutex<Option<Objc>>,
        last_scan: Mutex<Option<Instant>>,
        frame: AtomicU64,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "KassiberScannerFrames"]
        #[ivars = Arc<Shared>]
        struct Frames;

        impl Frames {
            #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
            fn did_output(&self, _output: *const AnyObject, sample: *const c_void, _connection: *const AnyObject) {
                self.ivars().frame(sample);
            }
        }
    );

    impl Frames {
        fn new(shared: Arc<Shared>) -> Retained<Self> {
            let this = Self::alloc().set_ivars(shared);
            unsafe { msg_send![super(this), init] }
        }
    }

    impl Shared {
        /// One captured frame: shown, and every 125 ms decoded.
        fn frame(&self, sample: *const c_void) {
            if sample.is_null() {
                return;
            }
            let pixels = unsafe { CMSampleBufferGetImageBuffer(sample) };
            if pixels.is_null() {
                return;
            }
            let id = self.frame.fetch_add(1, Ordering::Relaxed);
            if let Ok(buf) = unsafe { rune::native_surface::macos::PixelBuffer::from_raw(pixels) } {
                let _ = self.surface.push(buf, id);
            }
            let now = Instant::now();
            {
                let mut last = self.last_scan.lock().unwrap();
                if last.is_some_and(|l| now.duration_since(l) < SCAN_EVERY) {
                    return;
                }
                *last = Some(now);
            }
            for text in self.decode(pixels) {
                self.inbox.push(Event::Decoded(text));
            }
        }

        /// The QR texts in the frame's scan region.
        fn decode(&self, pixels: *mut c_void) -> Vec<String> {
            let Some(ci_image) = AnyClass::get(c"CIImage") else {
                return vec![];
            };
            let guard = self.detector.lock().unwrap();
            let Some(detector) = guard.as_ref() else {
                return vec![];
            };
            unsafe {
                let image: *mut AnyObject = msg_send![ci_image, imageWithCVPixelBuffer: pixels];
                if image.is_null() {
                    return vec![];
                }
                let extent: NSRect = msg_send![image, extent];
                let (x, y, w, h) = scan_region(extent.size.width, extent.size.height);
                let rect = NSRect::new(
                    objc2_foundation::NSPoint::new(extent.origin.x + x, extent.origin.y + y),
                    objc2_foundation::NSSize::new(w, h),
                );
                let cropped: *mut AnyObject = msg_send![image, imageByCroppingToRect: rect];
                if cropped.is_null() {
                    return vec![];
                }
                let features: *mut AnyObject = msg_send![&*detector.0, featuresInImage: cropped];
                if features.is_null() {
                    return vec![];
                }
                let count: usize = msg_send![features, count];
                let mut out = Vec::new();
                for i in 0..count {
                    let f: *mut AnyObject = msg_send![features, objectAtIndex: i];
                    let s: *mut NSString = msg_send![f, messageString];
                    if let Some(s) = s.as_ref() {
                        out.push(s.to_string());
                    }
                }
                out
            }
        }
    }

    /// A running capture session and what keeps it fed.
    pub struct Session {
        session: Arc<Mutex<Option<Objc>>>,
        /// Set when the dialog let go of the camera; a start still under way then stops the
        /// session it just began instead of leaving the camera on.
        dropped: Arc<std::sync::atomic::AtomicBool>,
        _frames: Retained<Frames>,
    }

    /// `stopRunning` blocks until capture stops; off the UI thread.
    fn stop(s: Objc) {
        std::thread::spawn(move || {
            let s: Objc = s;
            unsafe {
                let _: () = msg_send![&*s.0, stopRunning];
            }
        });
    }

    // SAFETY: see `Objc`; the delegate is only retained here so the output's weak reference lives.
    unsafe impl Send for Session {}

    impl Drop for Session {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
            if let Some(s) = self.session.lock().unwrap().take() {
                stop(s);
            }
        }
    }

    fn devices() -> Vec<(Device, Retained<AnyObject>)> {
        let Some(cls) = AnyClass::get(c"AVCaptureDevice") else {
            return vec![];
        };
        unsafe {
            let list: *mut AnyObject = msg_send![cls, devicesWithMediaType: AVMediaTypeVideo];
            if list.is_null() {
                return vec![];
            }
            let count: usize = msg_send![list, count];
            (0..count)
                .filter_map(|i| {
                    let d: *mut AnyObject = msg_send![list, objectAtIndex: i];
                    let d = Retained::retain(d)?;
                    let id: *mut NSString = msg_send![&*d, uniqueID];
                    let name: *mut NSString = msg_send![&*d, localizedName];
                    let s =
                        |p: *mut NSString| p.as_ref().map(|s| s.to_string()).unwrap_or_default();
                    Some((
                        Device {
                            id: s(id),
                            label: s(name),
                        },
                        d,
                    ))
                })
                .collect()
        }
    }

    /// The delegate, moved to the start thread.
    struct SendFrames(Retained<Frames>);
    // SAFETY: only retained and handed to AVFoundation from the start thread.
    unsafe impl Send for SendFrames {}

    impl Session {
        pub fn start(
            device: Option<String>,
            surface: rune::NativeSurface,
            inbox: Inbox,
        ) -> Option<Session> {
            let shared = Arc::new(Shared {
                surface,
                inbox: inbox.clone(),
                detector: Mutex::new(None),
                last_scan: Mutex::new(None),
                frame: AtomicU64::new(0),
            });
            let frames = Frames::new(shared.clone());
            let session: Arc<Mutex<Option<Objc>>> = Arc::default();
            let dropped: Arc<std::sync::atomic::AtomicBool> = Arc::default();
            let out = Session {
                session: session.clone(),
                dropped: dropped.clone(),
                _frames: frames.clone(),
            };
            let frames = SendFrames(frames);
            let begin = move || {
                if dropped.load(Ordering::SeqCst) {
                    return;
                }
                if let Err(e) = configure(device, &shared, frames, &session) {
                    shared.inbox.push(Event::Failed(e));
                }
                // The dialog closed while the session started: stop it now.
                if dropped.load(Ordering::SeqCst)
                    && let Some(s) = session.lock().unwrap().take()
                {
                    stop(s);
                }
            };
            let Some(av) = AnyClass::get(c"AVCaptureDevice") else {
                inbox.push(Event::Failed(NOT_FOUND.into()));
                return Some(out);
            };
            let status: isize =
                unsafe { msg_send![av, authorizationStatusForMediaType: AVMediaTypeVideo] };
            match status {
                AUTHORIZED => {
                    std::thread::spawn(begin);
                }
                NOT_DETERMINED => {
                    let begin = Mutex::new(Some(begin));
                    let ask = block2::RcBlock::new(move |granted: Bool| {
                        let Some(begin) = begin.lock().unwrap().take() else {
                            return;
                        };
                        if granted.as_bool() {
                            std::thread::spawn(begin);
                        } else {
                            inbox.push(Event::Failed(DENIED.into()));
                        }
                    });
                    unsafe {
                        let _: () = msg_send![av, requestAccessForMediaType: AVMediaTypeVideo, completionHandler: &*ask];
                    }
                }
                _ => inbox.push(Event::Failed(DENIED.into())),
            }
            Some(out)
        }
    }

    /// The session: input, BGRA output on a serial queue, the QR detector, then running.
    fn configure(
        device: Option<String>,
        shared: &Arc<Shared>,
        frames: SendFrames,
        slot: &Arc<Mutex<Option<Objc>>>,
    ) -> Result<(), String> {
        let all = devices();
        let chosen = device
            .as_deref()
            .and_then(|id| all.iter().find(|(d, _)| d.id == id))
            .or(all.first());
        let Some((info, dev)) = chosen else {
            return Err(NOT_FOUND.into());
        };
        shared.surface.set_mirrored(mirrored(&info.label));
        unsafe {
            let cls = |n: &std::ffi::CStr| AnyClass::get(n).ok_or_else(|| NOT_FOUND.to_owned());
            let mut err: *mut AnyObject = std::ptr::null_mut();
            let input: *mut AnyObject = msg_send![cls(c"AVCaptureDeviceInput")?, deviceInputWithDevice: &**dev, error: &mut err];
            if input.is_null() {
                let msg = err.as_ref().map(|e| {
                    let d: *mut NSString = msg_send![e, localizedDescription];
                    d.as_ref().map(|s| s.to_string()).unwrap_or_default()
                });
                return Err(msg
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| "Could not start the camera.".into()));
            }
            let session: Retained<AnyObject> = msg_send![cls(c"AVCaptureSession")?, new];
            let can: Bool = msg_send![&*session, canAddInput: input];
            if !can.as_bool() {
                return Err("Could not start the camera.".into());
            }
            let _: () = msg_send![&*session, addInput: input];
            let output: Retained<AnyObject> = msg_send![cls(c"AVCaptureVideoDataOutput")?, new];
            let number: *mut AnyObject = msg_send![cls(c"NSNumber")?, numberWithUnsignedInt: BGRA];
            let yes: *mut AnyObject = msg_send![cls(c"NSNumber")?, numberWithBool: true];
            let objects = [number, yes];
            let keys = [
                kCVPixelBufferPixelFormatTypeKey as *const NSString as *mut AnyObject,
                kCVPixelBufferMetalCompatibilityKey as *const NSString as *mut AnyObject,
            ];
            let settings: *mut AnyObject = msg_send![cls(c"NSDictionary")?, dictionaryWithObjects: objects.as_ptr(), forKeys: keys.as_ptr(), count: 2usize];
            let _: () = msg_send![&*output, setVideoSettings: settings];
            let _: () = msg_send![&*output, setAlwaysDiscardsLateVideoFrames: true];
            let queue = dispatch2::DispatchQueue::new("at.bitcoinaustria.kassiber.scanner", None);
            let queue_ptr = &*queue as *const dispatch2::DispatchQueue as *const AnyObject;
            let _: () = msg_send![&*output, setSampleBufferDelegate: &*frames.0, queue: queue_ptr];
            let can: Bool = msg_send![&*session, canAddOutput: &*output];
            if !can.as_bool() {
                return Err("Could not start the camera.".into());
            }
            let _: () = msg_send![&*session, addOutput: &*output];
            // The QR detector, high accuracy (qr-scanner's worker reads every frame it gets).
            let options: *mut AnyObject = msg_send![cls(c"NSDictionary")?, dictionaryWithObject: CIDetectorAccuracyHigh, forKey: CIDetectorAccuracy];
            let detector: *mut AnyObject = msg_send![cls(c"CIDetector")?, detectorOfType: CIDetectorTypeQRCode, context: std::ptr::null::<AnyObject>(), options: options];
            if let Some(d) = Retained::retain(detector) {
                *shared.detector.lock().unwrap() = Some(Objc(d));
            }
            let _: () = msg_send![&*session, startRunning];
            let running: Bool = msg_send![&*session, isRunning];
            if !running.as_bool() {
                return Err("Could not start the camera.".into());
            }
            *slot.lock().unwrap() = Some(Objc(session));
            // The queue lives as long as the output refers to it (libdispatch retains it).
            drop(queue);
        }
        shared
            .inbox
            .push(Event::Started(all.into_iter().map(|(d, _)| d).collect()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_region_is_the_centred_square() {
        assert_eq!(scan_region(1280.0, 720.0), (381.0, 101.0, 518.0, 518.0));
        assert_eq!(scan_region(400.0, 400.0), (56.0, 56.0, 288.0, 288.0));
    }

    #[test]
    fn user_facing_cameras_mirror() {
        assert!(mirrored("FaceTime HD Camera"));
        assert!(mirrored("Front Camera"));
        assert!(!mirrored("USB Video Device"));
    }
}
