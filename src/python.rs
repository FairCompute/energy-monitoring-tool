use crate::collectors::{NvidiaGpu, Rapl};
use crate::energy_group::{EnergyCollector, EnergyGroup};
use crate::utils::errors::MonitoringError;
use polars::prelude::DataFrame;
use pyo3::exceptions::{PyRuntimeError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict, PyType};
use tokio::runtime::{Builder, Runtime};

fn to_py_err(err: MonitoringError) -> PyErr {
    PyRuntimeError::new_err(err.to_string())
}

fn build_runtime() -> PyResult<Runtime> {
    Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| PyRuntimeError::new_err(err.to_string()))
}

#[pyclass(name = "RaplCollector", module = "emt._rust")]
#[derive(Debug, Default)]
pub struct PyRaplCollector {
    rapl_path: Option<String>,
}

#[pymethods]
impl PyRaplCollector {
    #[new]
    #[pyo3(signature = (rapl_path=None))]
    fn new(rapl_path: Option<String>) -> Self {
        Self { rapl_path }
    }

    #[staticmethod]
    fn is_available() -> bool {
        Rapl::is_available()
    }
}

#[pyclass(name = "NvidiaGpuCollector", module = "emt._rust")]
#[derive(Debug)]
pub struct PyNvidiaGpuCollector {
    device_ids: Vec<u32>,
}

#[pymethods]
impl PyNvidiaGpuCollector {
    #[new]
    #[pyo3(signature = (device_ids=None))]
    fn new(device_ids: Option<Vec<u32>>) -> Self {
        Self {
            device_ids: device_ids.unwrap_or_else(|| vec![0]),
        }
    }

    #[staticmethod]
    fn is_available() -> bool {
        NvidiaGpu::is_available()
    }
}

enum PyEnergyGroupInner {
    Rapl(EnergyGroup<Rapl>),
    NvidiaGpu(EnergyGroup<NvidiaGpu>),
}

impl PyEnergyGroupInner {
    fn set_tracked_pids(&self, pids: Vec<u32>) {
        match self {
            Self::Rapl(group) => group.set_tracked_pids(pids),
            Self::NvidiaGpu(group) => group.set_tracked_pids(pids),
        }
    }

    fn commence(&mut self, runtime: &Runtime) -> Result<(), MonitoringError> {
        match self {
            Self::Rapl(group) => runtime.block_on(group.commence()),
            Self::NvidiaGpu(group) => runtime.block_on(group.commence()),
        }
    }

    fn poll_data(&mut self) {
        match self {
            Self::Rapl(group) => {
                group.poll_data();
            }
            Self::NvidiaGpu(group) => {
                group.poll_data();
            }
        }
    }

    fn shutdown(&mut self) -> Result<(), MonitoringError> {
        match self {
            Self::Rapl(group) => group.shutdown(),
            Self::NvidiaGpu(group) => group.shutdown(),
        }
    }

    fn is_running(&self) -> bool {
        match self {
            Self::Rapl(group) => group.is_running(),
            Self::NvidiaGpu(group) => group.is_running(),
        }
    }

    fn energy_trace(&self) -> &DataFrame {
        match self {
            Self::Rapl(group) => group.energy_trace(),
            Self::NvidiaGpu(group) => group.energy_trace(),
        }
    }

    fn total_consumed_energy(&self) -> f64 {
        match self {
            Self::Rapl(group) => group.total_consumed_energy(),
            Self::NvidiaGpu(group) => group.total_consumed_energy(),
        }
    }
}

fn energy_trace_to_py_dict(py: Python<'_>, trace: &DataFrame) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);

    if trace.is_empty() || trace.width() == 0 {
        dict.set_item("pid", Vec::<u32>::new())?;
        dict.set_item("device", Vec::<String>::new())?;
        dict.set_item("energy", Vec::<f64>::new())?;
        dict.set_item("timestamp", Vec::<i64>::new())?;
        return Ok(dict.into_any().unbind());
    }

    let pids = trace
        .column("pid")
        .map_err(|err| PyRuntimeError::new_err(err.to_string()))?
        .u32()
        .map_err(|err| PyRuntimeError::new_err(err.to_string()))?
        .iter()
        .flatten()
        .collect::<Vec<_>>();
    let devices = trace
        .column("device")
        .map_err(|err| PyRuntimeError::new_err(err.to_string()))?
        .str()
        .map_err(|err| PyRuntimeError::new_err(err.to_string()))?
        .iter()
        .flatten()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let energies = trace
        .column("energy")
        .map_err(|err| PyRuntimeError::new_err(err.to_string()))?
        .f64()
        .map_err(|err| PyRuntimeError::new_err(err.to_string()))?
        .iter()
        .flatten()
        .collect::<Vec<_>>();
    let timestamps = trace
        .column("timestamp")
        .map_err(|err| PyRuntimeError::new_err(err.to_string()))?
        .i64()
        .map_err(|err| PyRuntimeError::new_err(err.to_string()))?
        .iter()
        .flatten()
        .collect::<Vec<_>>();

    dict.set_item("pid", pids)?;
    dict.set_item("device", devices)?;
    dict.set_item("energy", energies)?;
    dict.set_item("timestamp", timestamps)?;
    Ok(dict.into_any().unbind())
}

#[pyclass(name = "EnergyGroup", module = "emt._rust")]
pub struct PyEnergyGroup {
    runtime: Runtime,
    inner: PyEnergyGroupInner,
}

impl PyEnergyGroup {
    fn with_inner(inner: PyEnergyGroupInner) -> PyResult<Self> {
        Ok(Self {
            runtime: build_runtime()?,
            inner,
        })
    }
}

#[pymethods]
impl PyEnergyGroup {
    #[classmethod]
    #[pyo3(signature = (collector, rate, pids=None, batch_size=None))]
    fn create(
        _cls: &Bound<'_, PyType>,
        collector: &Bound<'_, PyAny>,
        rate: f64,
        pids: Option<Vec<u32>>,
        batch_size: Option<usize>,
    ) -> PyResult<Self> {
        if let Ok(collector_ref) = collector.extract::<PyRef<'_, PyRaplCollector>>() {
            let group = EnergyGroup::new(
                Rapl::new(collector_ref.rapl_path.clone()),
                rate,
                batch_size,
            );
            let result = Self::with_inner(PyEnergyGroupInner::Rapl(group))?;
            if let Some(pids) = pids {
                result.inner.set_tracked_pids(pids);
            }
            return Ok(result);
        }

        if let Ok(collector_ref) = collector.extract::<PyRef<'_, PyNvidiaGpuCollector>>() {
            let group = EnergyGroup::new(
                NvidiaGpu::new(collector_ref.device_ids.clone()),
                rate,
                batch_size,
            );
            let result = Self::with_inner(PyEnergyGroupInner::NvidiaGpu(group))?;
            if let Some(pids) = pids {
                result.inner.set_tracked_pids(pids);
            }
            return Ok(result);
        }

        Err(PyTypeError::new_err(
            "collector must be an instance of RaplCollector or NvidiaGpuCollector",
        ))
    }

    fn set_tracked_pids(&self, pids: Vec<u32>) {
        self.inner.set_tracked_pids(pids);
    }

    fn commence(&mut self, py: Python<'_>) -> PyResult<()> {
        let runtime = &self.runtime;
        py.detach(|| self.inner.commence(runtime).map_err(to_py_err))
    }

    fn poll_data(&mut self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| {
            self.inner.poll_data();
            Ok(())
        })
    }

    fn shutdown(&mut self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| self.inner.shutdown().map_err(to_py_err))
    }

    fn is_running(&self) -> bool {
        self.inner.is_running()
    }

    fn total_energy(&self) -> f64 {
        self.inner.total_consumed_energy()
    }

    fn energy_trace(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        energy_trace_to_py_dict(py, self.inner.energy_trace())
    }
}

#[pymodule]
fn _rust(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyEnergyGroup>()?;
    module.add_class::<PyRaplCollector>()?;
    module.add_class::<PyNvidiaGpuCollector>()?;
    Ok(())
}
