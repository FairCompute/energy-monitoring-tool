<div align="right">
  <img src="https://raw.githubusercontent.com/FairCompute/energy-monitoring-tool/main/assets/philips.png" alt="EMT Logo" width="120" />
  <img src="https://raw.githubusercontent.com/FairCompute/energy-monitoring-tool/main/assets/logo.png" alt="EMT Logo" width="180" />
</div>


[![Lines of Code](https://sonarqube.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=ncloc&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)
[![Coverage](https://sonarqube.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=coverage&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)
[![Security Hotspots](https://sonarqube.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=security_hotspots&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)
[![Technical Debt](https://sonarqube.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=software_quality_maintainability_remediation_effort&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)  
[![Quality Gate Status](https://sonarqube.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=alert_status&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)
[![Reliability Rating](https://sonarqube.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=software_quality_reliability_rating&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)
[![Maintainability Rating](https://sonarqube.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=software_quality_maintainability_rating&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)
[![Security Rating](https://sonarqube.workstation-home.com/api/project_badges/measure?project=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045&metric=software_quality_security_rating&token=sqb_dfadb2a54f25b2b7d59a71f83d23336d43cdc3e2)](https://sonar-ci-3f7k9v82.workstation-home.com/dashboard?id=FairCompute_energy-monitoring-tool_0b11396c-f1bf-41be-910a-f93bbc56f045)
---

# Energy Monitoring Tool (EMT)

*Track and analyze energy usage of your software application(s) — lightweight, reliable and effortless to integrate.*

**EMT** is a lightweight tool capable of tracking and reporting the energy consumption of software systems with process-level granularity.
While particularly valuable for compute-intensive workloads like machine learning, it's designed for broad applicability across various use cases.
Our mission is to simplify and standardize monitoring and reporting of the energy usage of the the digital realm. By making it visible and accessible, EMT helps teams to reduce their environmental impact and advances digital sustainability.

## 🚀 Features

- Real-time energy utilization tracking.
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
import torch
from emt import EnergyMonitor

# Dummy function
def add_tensors_gpu():
    device = torch.device(device if torch.cuda.is_available() else "cpu")
    # Generate random data
    a = torch.randint(1, 100, (1000,), dtype=torch.int32, device=device)
    b = torch.randint(1, 100, (1000,), dtype=torch.int32, device=device)

    return a + b

# Create a context manager
with EnergyMonitor() as monitor:
    add_tensors_gpu()

print(f"energy consumption: {monitor.consumed_energy}")
```

Refer to the following folder for example codes:
📁 examples/

####

## ⚙️ Methodology

The EMT context manager spawns a separate thread to monitor energy usage for CPUs and GPUs at regular intervals. It also tracks the utilization of these resources by the monitored process. EMT then estimates the process's share of the total energy consumption by proportionally assigning energy usage based on the resource utilization of the process.  


![EMT Energy Attribution](https://raw.githubusercontent.com/FairCompute/energy-monitoring-tool/main/assets/energy_attribution.png)


## 🤝 Contributions

Want to help make EMT Tool even better? We welcome your contributions! Here's how you can get involved:
1.  **Fork** our repository.
2.  Create a new branch for your work: `git checkout -b feature/your-feature-name`
3.  Make your changes and **commit** them with a clear message: `git commit -m 'Add your feature'`
4.  **Push** your changes to your new branch: `git push origin feature/your-feature-name`
5.  Open a **pull request** to share your contribution with us.

Please ensure that your pull request includes a clear description of the changes you've made and why they are valuable.
To keep the EMT Tool's codebase strong, please ensure your pull requests meet these standards:
>Clear Descriptions: Briefly explain your changes and their value.  
>Code Quality: Pass Sonar linter checks to help us keep the code healthy.  
>Consistent Style: Follow PEP 8 guidelines for clear, consistent Python code.  
>Unit Tests (If Applicable): Include unit tests for new features or major changes.  

## 🚧 Work in Progress

EMT Tool is an ongoing project, and we are actively working to enhance its features and usability. If you encounter any issues or have suggestions, please open an issue on the GitHub repository.

## 📧 Contact

For any inquiries or discussions, feel free to reach out to us at [rameez.ismail@philips.com](mailto:rameez.ismail@philips.com)

Let's code responsibly and make a positive impact on the environment! 🌍✨

## Acknowledgment

This project was originally initiated at Philips and continues to be actively maintained by the Philips Responsible AI team. We extend our sincere gratitude to all current and former contributors at Philips whose expertise, vision, and commitment to sustainability have been instrumental in shaping the Energy Monitoring Tool (EMT).

Philips has a longstanding dedication to sustainability and energy efficiency, guided by its core principle of care for people and the planet. This project reflects those values by providing open-source tools that empower organizations to monitor, analyze, and optimize their energy consumption. By making EMT open source through FairCompute, Philips aims to foster a broader community effort around sustainable computing, enabling both industry and research to benefit from advanced, transparent energy monitoring solutions.

<div align="right">
  <img src="https://raw.githubusercontent.com/FairCompute/energy-monitoring-tool/main/assets/philips.png" alt="EMT Logo" width="60" />
</div>

