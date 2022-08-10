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

// Inspired by https://github.com/phil-opp/blog_os/blob/post-03/src/vga_buffer.rs
// from Philipp Oppermann

use core::fmt;

use atomic_refcell::AtomicRefCell;
use uart_16550::SerialPort;

// We use COM1 as it is the standard first serial port.
pub static PORT: AtomicRefCell<SerialPort> = AtomicRefCell::new(unsafe { SerialPort::new(0x3f8) });

pub struct Serial;
impl fmt::Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        PORT.borrow_mut().write_str(s)
    }
}

// 将宏进行了导出
#[macro_export]
// 定义一个过程宏
macro_rules! log {
    // 外层$表示重复匹配，这里重复匹配没有分隔符
    // 模式匹配tt，变量名是arg
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        // 条件编译，
        #[cfg(all(feature = "log-serial", not(test)))]
        // 特殊宏变量 $crate，该宏被其他crate使用的时候，自动解析
        writeln!($crate::serial::Serial, $($arg)*).unwrap();
        #[cfg(all(feature = "log-serial", test))]
        // $(...)* 中的代码会重复展开
        println!($($arg)*);
    }};
}
