use crate::energy_group::EnergyRecord;
use crate::monitor::DeviceEnergy;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregatedDeviceClass {
    Cpu,
    Dram,
    Gpu,
}

#[derive(Debug, Clone, Default)]
pub struct GroupedEnergyTick {
    pub system_total: DeviceEnergy,
    pub group_energy: HashMap<String, DeviceEnergy>,
    pub unattributed: DeviceEnergy,
}

pub fn classify_record_device(record: &EnergyRecord) -> AggregatedDeviceClass {
    if record.device.starts_with("nvidia:") {
        AggregatedDeviceClass::Gpu
    } else if record.device == "rapl:system:dram" {
        AggregatedDeviceClass::Dram
    } else {
        AggregatedDeviceClass::Cpu
    }
}

pub fn aggregate_energy_records(
    records: &[EnergyRecord],
    pid_to_group: &HashMap<u32, String>,
) -> GroupedEnergyTick {
    let mut system_total = DeviceEnergy::default();
    let mut group_energy: HashMap<String, DeviceEnergy> = HashMap::new();
    let mut groups_sum = DeviceEnergy::default();

    for record in records {
        let device_class = classify_record_device(record);
        accumulate_device_energy(&mut system_total, device_class, record.energy);

        if let Some(group_id) = pid_to_group.get(&record.pid) {
            let entry = group_energy.entry(group_id.clone()).or_default();
            accumulate_device_energy(entry, device_class, record.energy);
            accumulate_device_energy(&mut groups_sum, device_class, record.energy);
        }
    }

    let unattributed = system_total.saturating_sub(&groups_sum);

    GroupedEnergyTick {
        system_total,
        group_energy,
        unattributed,
    }
}

pub fn percentage_of_system(group: &DeviceEnergy, system_total: &DeviceEnergy) -> f64 {
    let system_total_joules = system_total.total();
    if system_total_joules <= 0.0 {
        0.0
    } else {
        group.total() / system_total_joules * 100.0
    }
}

fn accumulate_device_energy(
    device_energy: &mut DeviceEnergy,
    device_class: AggregatedDeviceClass,
    joules: f64,
) {
    match device_class {
        AggregatedDeviceClass::Cpu => device_energy.cpu_joules += joules,
        AggregatedDeviceClass::Dram => device_energy.dram_joules += joules,
        AggregatedDeviceClass::Gpu => device_energy.gpu_joules += joules,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pid: u32, device: &str, energy: f64) -> EnergyRecord {
        EnergyRecord {
            pid,
            timestamp: 0,
            device: device.to_string(),
            energy,
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-10,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn classifies_cpu_dram_and_gpu_devices() {
        assert_eq!(
            classify_record_device(&record(1, "rapl:socket:0:package", 1.0)),
            AggregatedDeviceClass::Cpu
        );
        assert_eq!(
            classify_record_device(&record(1, "rapl:system:psys", 1.0)),
            AggregatedDeviceClass::Cpu
        );
        assert_eq!(
            classify_record_device(&record(1, "rapl:system:dram", 1.0)),
            AggregatedDeviceClass::Dram
        );
        assert_eq!(
            classify_record_device(&record(1, "nvidia:gpu:0", 1.0)),
            AggregatedDeviceClass::Gpu
        );
    }

    #[test]
    fn sums_multiple_pids_into_same_group() {
        let records = vec![
            record(101, "rapl:socket:0:package", 2.0),
            record(102, "rapl:system:dram", 0.5),
            record(102, "nvidia:gpu:0", 3.0),
        ];
        let pid_to_group =
            HashMap::from([(101, "compiler".to_string()), (102, "compiler".to_string())]);

        let tick = aggregate_energy_records(&records, &pid_to_group);
        let group = tick.group_energy.get("compiler").unwrap();

        assert_close(group.cpu_joules, 2.0);
        assert_close(group.dram_joules, 0.5);
        assert_close(group.gpu_joules, 3.0);
        assert_close(tick.system_total.total(), 5.5);
        assert_close(tick.unattributed.total(), 0.0);
    }

    #[test]
    fn unmapped_pid_contributes_to_unattributed() {
        let records = vec![
            record(101, "rapl:socket:0:package", 2.0),
            record(999, "rapl:system:dram", 0.75),
            record(999, "nvidia:gpu:0", 1.25),
        ];
        let pid_to_group = HashMap::from([(101, "compiler".to_string())]);

        let tick = aggregate_energy_records(&records, &pid_to_group);

        assert_close(tick.system_total.cpu_joules, 2.0);
        assert_close(tick.system_total.dram_joules, 0.75);
        assert_close(tick.system_total.gpu_joules, 1.25);
        assert_close(tick.unattributed.cpu_joules, 0.0);
        assert_close(tick.unattributed.dram_joules, 0.75);
        assert_close(tick.unattributed.gpu_joules, 1.25);
        assert!(!tick.group_energy.contains_key("999"));
    }

    #[test]
    fn percentage_math_handles_zero_and_expected_percent() {
        let group = DeviceEnergy {
            cpu_joules: 2.0,
            dram_joules: 1.0,
            gpu_joules: 1.0,
        };
        let zero_total = DeviceEnergy::default();
        let system_total = DeviceEnergy {
            cpu_joules: 4.0,
            dram_joules: 2.0,
            gpu_joules: 2.0,
        };

        assert_close(percentage_of_system(&group, &zero_total), 0.0);
        assert_close(percentage_of_system(&group, &system_total), 50.0);
    }
}
