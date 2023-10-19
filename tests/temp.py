import threading
from emt import EnergyMeter
from emt.groups import IntelCPU
from time import sleep

# Assuming you have the EnergyMeter class defined as in your initial code

# Create an instance of the EnergyMeter class
energy_meter = EnergyMeter(power_groups=[IntelCPU()])  # Provide your PowerGroup objects
print(f'executing from main-thread: id: {id(energy_meter)}')


# Create a separate thread and start it
energy_meter_thread = threading.Thread(target=lambda :energy_meter.run())
energy_meter_thread.start()

# Now, the EnergyMeter will run in the background thread without blocking the main thread.
# You can continue doing other tasks in the main thread.
for k in range(10):
    print(k)
    sleep(1)
energy_meter.conclude()
energy_meter_thread.join()