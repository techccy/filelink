//! macOS 剪贴板文件路径读取（Finder 复制的文件 → NSPasteboard 里的 NSURL）。
//! 直接走 objc2 msg_send，避免绑定 crate 的特性矩阵。

#[cfg(target_os = "macos")]
pub fn clipboard_file_paths() -> Vec<String> {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;

    objc2::rc::autoreleasepool(|_| unsafe {
        let pb: Retained<AnyObject> = objc2::msg_send![objc2::class!(NSPasteboard), generalPasteboard];
        let url_cls: *const objc2::runtime::AnyClass = objc2::class!(NSURL);
        let classes: Retained<AnyObject> =
            objc2::msg_send![objc2::class!(NSArray), arrayWithObject: url_cls];
        let objs: Option<Retained<AnyObject>> = objc2::msg_send![
            &pb,
            readObjectsForClasses: &*classes as *const AnyObject,
            share: std::ptr::null::<AnyObject>()
        ];

        let mut out = Vec::new();
        if let Some(arr) = objs {
            let count: usize = objc2::msg_send![&arr, count];
            for i in 0..count {
                let obj: Option<Retained<AnyObject>> = objc2::msg_send![&arr, objectAtIndex: i];
                let Some(obj) = obj else { continue };
                let is_file: bool = objc2::msg_send![&obj, isFileURL];
                if !is_file {
                    continue;
                }
                let path: Option<Retained<AnyObject>> = objc2::msg_send![&obj, path];
                if let Some(p) = path {
                    let utf8: *const std::ffi::c_char = objc2::msg_send![&p, UTF8String];
                    if !utf8.is_null() {
                        out.push(
                            std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned(),
                        );
                    }
                }
            }
        }
        out
    })
}

#[cfg(not(target_os = "macos"))]
pub fn clipboard_file_paths() -> Vec<String> {
    Vec::new()
}
