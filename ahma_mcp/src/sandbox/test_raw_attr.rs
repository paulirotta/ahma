use std::os::windows::process::CommandExt;
use std::process::Command;
struct SecurityCapabilities { sid: *mut std::ffi::c_void }
fn main() {
    let mut cmd = Command::new("cmd");
    let caps = SecurityCapabilities { sid: std::ptr::null_mut() };
    cmd.raw_attribute(0x00020009, &caps as *const _ as *mut _); // PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES is 0x00020009
    
    println!("cmd: {:?}", cmd);
}
