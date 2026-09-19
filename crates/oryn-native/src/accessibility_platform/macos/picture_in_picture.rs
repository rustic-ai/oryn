//! The main-thread adapter for actor-owned decoded PiP surfaces.
use super::*;
struct Window {
    window: Id,
    view: Id,
    revision: u64,
    visible: bool,
}
#[derive(Default)]
pub(super) struct Host {
    windows: BTreeMap<String, Window>,
}
impl Host {
    pub(super) fn pump(&mut self) -> Result<(), String> {
        unsafe {
            let pool = object(class("NSAutoreleasePool"), "new");
            struct Pool(Id);
            impl Drop for Pool {
                fn drop(&mut self) {
                    unsafe { void(self.0, "drain") }
                }
            }
            let _pool = Pool(pool);
            let surfaces = crate::media_capture::picture_in_picture::snapshots();
            let retired = self
                .windows
                .keys()
                .filter(|id| !surfaces.iter().any(|surface| &surface.id == *id))
                .cloned()
                .collect::<Vec<_>>();
            for id in retired {
                if let Some(window) = self.windows.remove(&id) {
                    void(window.window, "close");
                    void(window.view, "release");
                    void(window.window, "release");
                }
            }
            for surface in surfaces {
                if !self.windows.contains_key(&surface.id) {
                    let init: unsafe extern "C" fn(Id, Sel, Rect, usize, usize, i8) -> Id =
                        std::mem::transmute(objc_msgSend as *const ());
                    let window = init(
                        object(class("NSPanel"), "alloc"),
                        sel("initWithContentRect:styleMask:backing:defer:"),
                        Rect {
                            origin: Point { x: 120., y: 120. },
                            size: Size {
                                width: surface.width as f64,
                                height: surface.height as f64,
                            },
                        },
                        1 | 2 | 8 | 128,
                        2,
                        0,
                    );
                    if window.is_null() {
                        return Err("PiP native window allocation".into());
                    }
                    let boolean: unsafe extern "C" fn(Id, Sel, i8) =
                        std::mem::transmute(objc_msgSend as *const ());
                    boolean(window, sel("setReleasedWhenClosed:"), 0);
                    boolean(window, sel("setHidesOnDeactivate:"), 0);
                    let level: unsafe extern "C" fn(Id, Sel, isize) =
                        std::mem::transmute(objc_msgSend as *const ());
                    level(window, sel("setLevel:"), 3);
                    let init_view: unsafe extern "C" fn(Id, Sel, Rect) -> Id =
                        std::mem::transmute(objc_msgSend as *const ());
                    let view = init_view(
                        object(class("NSImageView"), "alloc"),
                        sel("initWithFrame:"),
                        Rect {
                            origin: Point { x: 0., y: 0. },
                            size: Size {
                                width: surface.width as f64,
                                height: surface.height as f64,
                            },
                        },
                    );
                    if view.is_null() {
                        void(window, "close");
                        void(window, "release");
                        return Err("PiP native image view allocation".into());
                    }
                    level(view, sel("setImageScaling:"), 3);
                    void_arg(window, "setContentView:", view);
                    void_arg(window, "setTitle:", string("Picture in Picture"));
                    self.windows.insert(
                        surface.id.clone(),
                        Window {
                            window,
                            view,
                            revision: u64::MAX,
                            visible: false,
                        },
                    );
                }
                let output = self.windows.get_mut(&surface.id).unwrap();
                let visible: unsafe extern "C" fn(Id, Sel) -> i8 =
                    std::mem::transmute(objc_msgSend as *const ());
                if output.visible
                    && surface.visible
                    && visible(output.window, sel("isVisible")) == 0
                {
                    crate::media_capture::picture_in_picture::user_closed(&surface.id);
                    continue;
                }
                if surface.visible && !output.visible {
                    void_arg(output.window, "orderFront:", std::ptr::null_mut());
                } else if !surface.visible && output.visible {
                    void_arg(output.window, "orderOut:", std::ptr::null_mut());
                }
                output.visible = surface.visible;
                // NSView frame return is an aggregate; use the correct x86_64 stret ABI.
                #[cfg(target_arch = "aarch64")]
                let rect = {
                    let frame: unsafe extern "C" fn(Id, Sel) -> Rect =
                        std::mem::transmute(objc_msgSend as *const ());
                    frame(output.view, sel("frame"))
                };
                #[cfg(target_arch = "x86_64")]
                let rect = {
                    unsafe extern "C" {
                        fn objc_msgSend_stret();
                    }
                    let frame: unsafe extern "C" fn(*mut Rect, Id, Sel) =
                        std::mem::transmute(objc_msgSend_stret as *const ());
                    let mut rect = Rect {
                        origin: Point { x: 0., y: 0. },
                        size: Size {
                            width: 0.,
                            height: 0.,
                        },
                    };
                    frame(&mut rect, output.view, sel("frame"));
                    rect
                };
                crate::media_capture::picture_in_picture::resized(
                    &surface.id,
                    rect.size.width.max(1.) as u32,
                    rect.size.height.max(1.) as u32,
                );
                if output.revision == surface.revision || surface.pixels.is_empty() {
                    continue;
                }
                let bitmap: unsafe extern "C" fn(
                    Id,
                    Sel,
                    *mut *mut u8,
                    isize,
                    isize,
                    isize,
                    isize,
                    i8,
                    i8,
                    Id,
                    usize,
                    isize,
                    isize,
                ) -> Id = std::mem::transmute(objc_msgSend as *const ());
                let rep = bitmap(
                    object(class("NSBitmapImageRep"), "alloc"),
                    sel(
                        "initWithBitmapDataPlanes:pixelsWide:pixelsHigh:bitsPerSample:samplesPerPixel:hasAlpha:isPlanar:colorSpaceName:bitmapFormat:bytesPerRow:bitsPerPixel:",
                    ),
                    std::ptr::null_mut(),
                    surface.pixel_width as isize,
                    surface.pixel_height as isize,
                    8,
                    4,
                    1,
                    0,
                    string("NSDeviceRGBColorSpace"),
                    0,
                    surface.pixel_width as isize * 4,
                    32,
                );
                if rep.is_null() {
                    return Err("PiP native bitmap allocation".into());
                }
                let data: unsafe extern "C" fn(Id, Sel) -> *mut u8 =
                    std::mem::transmute(objc_msgSend as *const ());
                let data = data(rep, sel("bitmapData"));
                if data.is_null() {
                    void(rep, "release");
                    return Err("PiP native bitmap data".into());
                }
                std::ptr::copy_nonoverlapping(surface.pixels.as_ptr(), data, surface.pixels.len());
                let init: unsafe extern "C" fn(Id, Sel, Size) -> Id =
                    std::mem::transmute(objc_msgSend as *const ());
                let image = init(
                    object(class("NSImage"), "alloc"),
                    sel("initWithSize:"),
                    Size {
                        width: surface.pixel_width as f64,
                        height: surface.pixel_height as f64,
                    },
                );
                if image.is_null() {
                    void(rep, "release");
                    return Err("PiP native image allocation".into());
                }
                void_arg(image, "addRepresentation:", rep);
                void_arg(output.view, "setImage:", image);
                void(image, "release");
                void(rep, "release");
                output.revision = surface.revision;
            }
            Ok(())
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let pool = unsafe { object(class("NSAutoreleasePool"), "new") };
        for (id, window) in std::mem::take(&mut self.windows) {
            crate::media_capture::picture_in_picture::user_closed(&id);
            unsafe {
                void(window.window, "close");
                void(window.view, "release");
                void(window.window, "release");
            }
        }
        unsafe {
            void(pool, "drain");
        }
    }
}
