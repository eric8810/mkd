//! 系统文件拖入：给 GPUI 的 NSView 注册 NSDraggingDestination。
//! GPUI 0.2 无系统拖放事件，这里用 objc2 直接桥接 AppKit。

use std::path::PathBuf;
use std::sync::{Mutex, Once};

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::msg_send;
use objc2::MainThreadMarker;
use objc2::{define_class, ClassType, MainThreadOnly};
use objc2_app_kit::{
    NSDragOperation, NSDraggingDestination, NSDraggingInfo, NSPasteboard, NSPasteboardType,
    NSView,
};
use objc2_foundation::{NSArray, NSString, NSObject};

// 持有目标实例（leak，防释放），主线程使用。
static DROP_TARGET: Mutex<Option<usize>> = Mutex::new(None);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct FileDropTarget;

    unsafe impl NSObjectProtocol for FileDropTarget {}

    unsafe impl NSDraggingDestination for FileDropTarget {
        #[unsafe(method(draggingEntered:))]
        unsafe fn draggingEntered(
            &self,
            _sender: &ProtocolObject<dyn NSDraggingInfo>,
        ) -> NSDragOperation {
            NSDragOperation::Copy
        }

        #[unsafe(method(performDragOperation:))]
        unsafe fn performDragOperation(
            &self,
            sender: &ProtocolObject<dyn NSDraggingInfo>,
        ) -> bool {
            let pb = sender.draggingPasteboard();
            let ft = NSString::from_str("public.file-url");
            if let Some(url) = pb.stringForType(&ft) {
                let s = url.to_string();
                let path = s.strip_prefix("file://").unwrap_or(&s);
                let decoded = percent_decode(path);
                *crate::PENDING_PATH.lock().unwrap() = Some(PathBuf::from(decoded));
            }
            true
        }
    }
);

/// 简单 URL 百分号解码（%20 等）。
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

static INSTALLED: Once = Once::new();

/// 把文件拖入目标注册到内容视图（每进程一次）。
///
/// `ns_view` 来自 GPUI `Window` 的 `raw_window_handle`（AppKit 变体）。
pub fn ensure_installed(ns_view: *mut std::ffi::c_void) {
    INSTALLED.call_once(|| {
        unsafe {
            autoreleasepool(|_| {
                let marker = MainThreadMarker::new().unwrap();
                let _ = marker;
                let cls = FileDropTarget::class();
                let target: Retained<FileDropTarget> = msg_send![cls, new];
                let raw = Retained::into_raw(target) as usize;
                *DROP_TARGET.lock().unwrap() = Some(raw);
                let view = &*(ns_view as *const NSView);
                let t1 = NSPasteboardType::from_str("public.file-url");
                let t2 = NSPasteboardType::from_str("public.url");
                let t3 = NSPasteboardType::from_str("NSFilenamesPboardType");
                let types = NSArray::from_slice(&[&*t1, &*t2, &*t3]);
                view.registerForDraggedTypes(&types);
            });
        }
    });
}
