// Copyright © 2019 Intel Corporation
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![feature(alloc_error_handler)]
#![feature(stmt_expr_attributes)]
#![feature(slice_take)]
// 条件编译，不是test的情况，设置 no_std。如果我们试图从std导入或使用依赖于std的crate，编译器将引发错
// 告诉 rustc 只有在禁用 test 标志时才编译为 no-std Rust，这样我们就可以在测试中使用 std，就像 std Rust 一样。
#![cfg_attr(not(test), no_std)]
// 需要重新实现最开始的入口函数，这个函数依赖于具体的操作系统。在这个之前先需要禁用默认的 main 函数
#![cfg_attr(not(test), no_main)]
#![cfg_attr(test, allow(unused_imports, dead_code))]
#![cfg_attr(not(feature = "log-serial"), allow(unused_variables, unused_imports))]

use core::panic::PanicInfo;

use x86_64::{
    instructions::hlt,
    registers::control::{Cr0, Cr0Flags, Cr4, Cr4Flags},
};

// 引入其他module定义的宏
#[macro_use]
mod serial;

#[macro_use]
mod common;

// mod asm 会引入 asm/mod.rs
#[cfg(not(test))]
mod asm;
mod block;
mod boot;
mod bzimage;
mod coreboot;
mod delay;
mod efi;
mod fat;
mod gdt;
#[cfg(all(test, feature = "integration_tests"))]
mod integration;
mod loader;
mod mem;
mod paging;
mod part;
mod pci;
mod pe;
mod pvh;
mod rtc;
mod virtio;

// 因为设置了no_std，所以需要自己设置一个panic异常处理函数
#[cfg(all(not(test), feature = "log-panic"))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    log!("PANIC: {}", info);
    loop {
        hlt()
    }
}

#[cfg(all(not(test), not(feature = "log-panic")))]
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {}
}

// Enable SSE2 for XMM registers (needed for EFI calling)
fn enable_sse() {
    let mut cr0 = Cr0::read();
    cr0.remove(Cr0Flags::EMULATE_COPROCESSOR);
    cr0.insert(Cr0Flags::MONITOR_COPROCESSOR);
    unsafe { Cr0::write(cr0) };
    let mut cr4 = Cr4::read();
    cr4.insert(Cr4Flags::OSFXSR);
    cr4.insert(Cr4Flags::OSXMMEXCPT_ENABLE);
    unsafe { Cr4::write(cr4) };
}

const VIRTIO_PCI_VENDOR_ID: u16 = 0x1af4;
const VIRTIO_PCI_BLOCK_DEVICE_ID: u16 = 0x1042;

// dyn是和trait配合一起用的
fn boot_from_device(device: &mut block::VirtioBlockDevice, info: &dyn boot::Info) -> bool {
    if let Err(err) = device.init() {
        log!("Error configuring block device: {:?}", err);
        return false;
    }
    log!(
        "Virtio block device configured. Capacity: {} sectors",
        device.get_capacity()
    );

    // 使用match 语法
    let (start, end) = match part::find_efi_partition(device) {
        Ok(p) => p,
        Err(err) => {
            log!("Failed to find EFI partition: {:?}", err);
            return false;
        }
    };
    log!("Found EFI partition");

    let mut f = fat::Filesystem::new(device, start, end);
    if let Err(err) = f.init() {
        log!("Failed to create filesystem: {:?}", err);
        return false;
    }
    log!("Filesystem ready");

    match loader::load_default_entry(&f, info) {
        Ok(mut kernel) => {
            log!("Jumping to kernel");
            kernel.boot();
            return true;
        }
        Err(err) => log!("Error loading default entry: {:?}", err),
    }

    log!("Using EFI boot.");
    let mut file = match f.open("/EFI/BOOT/BOOTX64 EFI") {
        Ok(file) => file,
        Err(err) => {
            log!("Failed to load default EFI binary: {:?}", err);
            return false;
        }
    };
    log!("Found bootloader (BOOTX64.EFI)");

    let mut l = pe::Loader::new(&mut file);
    let load_addr = 0x20_0000;
    let (entry_addr, load_addr, size) = match l.load(load_addr) {
        Ok(load_info) => load_info,
        Err(err) => {
            log!("Error loading executable: {:?}", err);
            return false;
        }
    };

    log!("Executable loaded");
    efi::efi_exec(entry_addr, load_addr, size, info, &f, device);
    true
}

#[no_mangle]
#[cfg(not(feature = "coreboot"))]
pub extern "C" fn rust64_start(rdi: &pvh::StartInfo) -> ! {
    serial::PORT.borrow_mut().init();

    enable_sse();
    paging::setup();

    main(rdi)
}

// 使用关键字extern，可以创建一个允许其他语言调用 Rust 函数的接口。
// extern "C"，指定使用 C-ABI，类似extern fn foo()，无论 C 编译器支持哪种默认设置。
// 属性no_mangle，用来关闭 Rust 的名称修改（name mangling）功能
#[no_mangle]
#[cfg(feature = "coreboot")]
pub extern "C" fn rust64_start() -> ! {
    serial::PORT.borrow_mut().init();

    enable_sse();
    paging::setup();

    let info = coreboot::StartInfo::default();

    main(&info)
}

fn main(info: &dyn boot::Info) -> ! {
    log!("\nBooting with {}", info.name());

    pci::print_bus();

    pci::with_devices(
        VIRTIO_PCI_VENDOR_ID,
        VIRTIO_PCI_BLOCK_DEVICE_ID,
        // 闭包参数pci_device
        |pci_device| {
            let mut pci_transport = pci::VirtioPciTransport::new(pci_device);
            let mut device = block::VirtioBlockDevice::new(&mut pci_transport);
            boot_from_device(&mut device, info)
        },
    );

    panic!("Unable to boot from any virtio-blk device")
}
