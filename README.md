
# Energy Monitoring Tool (EMT) <img src="assets/logo.png" alt="EMT Logo" width="60"/>

**EMT** is a lightweight, Python-based tool that tracks the energy consumption of applications with process-level granularity. Designed with a strong focus on machine learning, it enables monitoring of the energy usage of training and inference for large deep learning models across diverse computing environments. EMT is framework-agnostic and generates process-level energy consumption log. The repository provides concrete examples of how to track energy consumption in various scenarios. EMT simplifies and democratizes energy monitoring, enabling developers and operations teams to actively reduce the environmental footprint thus advancing digital sustainability initiatives. 


## 🚀 Features

- Real-time energy utilization tracking.
- Device-level breakdown of energy consumption.
- Enegy/Power attribution to a process of interest in a multi-process shared resource setting.
- Modular and extendable software architecture, currently supports following powergroups:
  - CPU(s) with RAPL capabilites.
  - Nvidia GPUs.
- Visualization interface for energy data using TensorBoard,  making it easy to analyze energy usage trends.

  #### Supported Platforms
  - Linux
  

> Road Map & Future Releases:
>
> - Inline prediction of energy consumption.
> - Environmentally conscious coding tips.
> - Virtual CPU(s) covered by Teads dataset.
> - Add support for Windows through PCM/OpenHardwareMonitor

## 🌍 Why EMT?

In the era of climate awareness, it's essential for developers to contribute to a sustainable future. EMT Tool empowers you to make informed decisions about your code's impact on the environment and take steps towards writing more energy-efficient software.

## 🛠️ Getting Started

Install latest emt package from philips `dl-innersource-pypi` repository

That's it! You're now ready to analyze the energy footprint of your code.

### _Usage_:

> We currently plan to support three modes of usage: Python Context Managers, Keras Callbacks and CLI.
> The _callbacks_ focus on working with popular ML library Keras, the python _context manager_ mode can
> be easily integrated into any python code, while the _CLI_ mode allows usage of the tool for
> command-line application that are not writtern in python.  
> **Only Python Context Manager Mode is implemented so far!**

#### Using Python Context Managers

```python
import timeit
import logging
import torch
import emt
from emt import EnergyMonitor
from emt.utils import CSVRecorder, TensorboardRecorder

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
    # EMT will track energy for any code under this context
    # Repeat the addition 10000 times
    execution_time = timeit.timeit(add_tensors_gpu, number=1000000)

print(f"execution time: {execution_time:.2f} Seconds.")
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
