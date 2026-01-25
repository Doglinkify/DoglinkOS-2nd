use aml::AmlName;
use x86_64::instructions::port::Port;

use crate::acpi::{AML_CONTEXT, FADT};

pub fn poweroff() -> ! {
    let aml_context = AML_CONTEXT.lock();
    let slp_typa = match aml_context
        .namespace
        .get_by_path(&AmlName::from_str("\\_S5").unwrap())
        .unwrap()
    {
        aml::AmlValue::Package(ref values) => values[0].as_integer(&aml_context).unwrap() as u16,
        _ => unreachable!(),
    };
    loop {
        let pm1a = FADT.pm1a_control_block().unwrap();
        unsafe { Port::new(pm1a.address as u16).write(slp_typa | (1 << 13)) }
    }
}
pub fn reboot() -> ! {
    loop {
        let reset = FADT.reset_register().unwrap();
        unsafe { Port::new(reset.address as u16).write(FADT.reset_value) }
    }
}
