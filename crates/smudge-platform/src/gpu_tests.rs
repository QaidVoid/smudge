//! Tests for the automatic choice between a real GPU and a software
//! rasterizer.
//!
//! The adapter facts are the reports observed on real machines: the NVIDIA
//! and RADV adapters of a desktop, and llvmpipe on the GL backend of wgpu,
//! which reports a device type of CPU and a name of `llvmpipe (...)` on
//! recent Mesa and a device type of Other on older ones.

use smudge_gpu::wgpu::{AdapterInfo, Backend, DeviceType};

use super::adapter_is_software;

/// Returns adapter info with the given report.
fn info(name: &str, driver: &str, device_type: DeviceType) -> AdapterInfo {
    AdapterInfo {
        name: String::from(name),
        driver: String::from(driver),
        ..AdapterInfo::new(device_type, Backend::Vulkan)
    }
}

#[test]
fn a_cpu_device_type_is_software() {
    let adapters = [
        info("llvmpipe (LLVM 22.1.8, 256 bits)", "", DeviceType::Cpu),
        info("Unknown", "", DeviceType::Cpu),
    ];
    for adapter in &adapters {
        assert!(
            adapter_is_software(adapter),
            "{adapter:?} must count as software"
        );
    }
}

#[test]
fn the_software_rasterizer_names_are_software_at_any_case() {
    let adapters = [
        // Older Mesa reports llvmpipe on GL as a device type of Other.
        info("llvmpipe (LLVM 15.0.7, 256 bits)", "", DeviceType::Other),
        info("softpipe", "", DeviceType::Other),
        info("SwiftShader Device (LLVM 17.0.6)", "", DeviceType::Cpu),
        info("Microsoft Basic Render Driver", "", DeviceType::Cpu),
    ];
    for adapter in &adapters {
        assert!(
            adapter_is_software(adapter),
            "{adapter:?} must count as software"
        );
    }
}

#[test]
fn a_driver_name_can_mark_the_adapter_software() {
    let adapter = info("Virtio-GPU Venus (0)", "lavapipe", DeviceType::VirtualGpu);
    assert!(adapter_is_software(&adapter));
}

#[test]
fn real_gpu_adapters_are_not_software() {
    let adapters = [
        info(
            "NVIDIA GeForce GTX 1060 6GB",
            "NVIDIA",
            DeviceType::DiscreteGpu,
        ),
        info(
            "AMD Ryzen 7 7700 8-Core Processor (RADV RAPHAEL_MENDOCINO)",
            "radv",
            DeviceType::IntegratedGpu,
        ),
        // A paravirtualized GPU stays hardware-backed, and the GL backend of
        // wgpu reports a device type of Other.
        info("virtio-gpu (Venus)", "venus", DeviceType::VirtualGpu),
        info(
            "NVIDIA GeForce GTX 1060 6GB/PCIe/SSE2",
            "",
            DeviceType::Other,
        ),
        info("Apple M1", "Apple M1", DeviceType::IntegratedGpu),
    ];
    for adapter in &adapters {
        assert!(
            !adapter_is_software(adapter),
            "{adapter:?} must count as a real GPU"
        );
    }
}
