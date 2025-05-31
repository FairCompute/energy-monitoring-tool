<div align="right">
  <img src="https://raw.githubusercontent.com/FairCompute/energy-monitoring-tool/main/assets/logo.png" alt="EMT Logo" width="200" />
</div>



[![Coverage](https://sonar-ci-3f7k9v82.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=coverage&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)
[![Lines of Code](https://sonar-ci-3f7k9v82.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=ncloc&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)
[![Security Hotspots](https://sonar-ci-3f7k9v82.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=security_hotspots&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)
[![Technical Debt](https://sonar-ci-3f7k9v82.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=software_quality_maintainability_remediation_effort&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)  
[![Quality Gate Status](https://sonar-ci-3f7k9v82.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=alert_status&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)
[![Reliability Rating](https://sonar-ci-3f7k9v82.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=software_quality_reliability_rating&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)
[![Security Rating](https://sonar-ci-3f7k9v82.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=software_quality_security_rating&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)
[![Maintainability Rating](https://sonar-ci-3f7k9v82.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=software_quality_maintainability_rating&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)


# Energy Monitoring Tool (EMT)
*Track and analyze energy usage of your software application(s) — lightweight, accurate and scriptable.*


**EMT** is a lightweight tool capable of tracking and reporting the energy consumption of software applications with process-level granularity.
While especially useful for monitoring Machine Learning (ML) workloads, such as training and inference of large models, EMT is designed to work across a range of applications and use cases and therefore is not just limited to ML.

Our mission is to simplify and standardize monitoring and reporting of the energy usage of the digital solutions. By making it visible and accessible, EMT helps teams reduce the environmental impact of the digital realm and advances digital sustainability.

## 🚀 Features

- Real-time energy utilization tracking.lo
- Device-level breakdown of energy consumption.
- Energy/Power attribution to a process of interest in a multi-process shared resource setting.
- Modular and extendable software architecture, currently supports following powergroups:
  - CPU(s) with RAPL capabilities.
  - Nvidia GPUs.
- Visualization interface for energy data using TensorBoard,  making it easy to analyze energy usage trends.

## Supported Platforms

- Linux
  
> Road Map
>
> - Environmentally conscious coding tips.
> - Virtual CPU(s) covered by Teads dataset.
> - Add support for Windows through PCM/OpenHardwareMonitor

## 🌍 Why EMT?

In the era of climate awareness, it's essential for developers to contribute to a sustainable future. EMT Tool empowers you to make informed decisions about your code's impact on the environment and take steps towards writing more energy-efficient software.

## 🛠️ Getting Started

Install the latest [EMT package](https://pypi.org/project/emt/)  from the Python Package Index (PyPI):  

``` bash
pip install emt

# verify installation and the version
python -m emt --version
```

### *Usage*

> The tool supports two usage modes:
>
> - **Python Context Manager**  
>   Fully implemented and ideal for instrumenting Python code directly. This mode allows developers to wrap specific code blocks to measure energy consumption with precision.
> - **Command-Line Interface (CLI)**  
>   Designed to tag and monitor running application without modifying the code.  
>   *This mode is currently under active development and will be available soon.*

#### Using Python Context Managers

```python
import logging
import torch
import emt
from emt import EnergyMonitor

emt.setup_logger(
    log_dir="./logs/example/",
)

# Dummy function
def add_tensors_gpu():
    device = torch.device(device if torch.cuda.is_available() else "cpu")
    # Generate random data
    a = torch.randint(1, 100, (1000,), dtype=torch.int32, device=device)
    b = torch.randint(1, 100, (1000,), dtype=torch.int32, device=device)

    return a + b

# Create a context manager
with EnergyMonitor as monitor:
    add_tensors_gpu()

print(f"energy consumption: {monitor.total_consumed_energy:.2f} J")
print(f"energy consumption: {monitor.consumed_energy}")
```

Refer to the following folder for example codes:
📁 examples/

####

## ⚙️ Methodology

The EMT context manager spawns a separate thread to monitor energy usage for CPUs and GPUs at regular intervals. It also tracks the utilization of these resources by the monitored process. EMT then estimates the process's share of the total energy consumption by proportionally assigning energy usage based on the resource utilization of the process.

<div align="center">
  <img src="assets/emt_method.png" alt="EMT Methods Illustration" width="40%">
  <p><em>Figure: Overview of Utilized Energy/Power Calculation </em></p>
</div>

## 🤝 Contributions

We welcome contributions from the community to make EMT Tool even more robust and feature-rich. To contribute, follow these steps:

1. Fork the repository
2. Create a new branch: `git checkout -b feature/your-feature-name`
3. Make your changes and commit them: `git commit -m 'Add your feature'`
4. Push to the branch: `git push origin feature/your-feature-name`
5. Open a pull request

Please ensure that your pull request includes a clear description of the changes you've made and why they are valuable. Additionally, ensure that your code adheres to the project's coding standards.

## 🚧 Work in Progress

EMT Tool is an ongoing project, and we are actively working to enhance its features and usability. If you encounter any issues or have suggestions, please open an issue on the GitHub repository.

## 📧 Contact

For any inquiries or discussions, feel free to reach out to us at [rameez.ismail@philips.com](mailto:rameez.ismail@philips.com)

Let's code responsibly and make a positive impact on the environment! 🌍✨
