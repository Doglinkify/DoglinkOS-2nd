use crate::println;
use raw_cpuid::CpuId;

pub fn show_cpu_info() {
    let cpuid = CpuId::new();
    println!("[INFO] cpu: CPU Vendor: {}", cpuid.get_vendor_info().unwrap().as_str());
    println!(
        "[INFO] cpu: CPU Model Name: {}",
        cpuid.get_processor_brand_string().unwrap().as_str()
    );
}
