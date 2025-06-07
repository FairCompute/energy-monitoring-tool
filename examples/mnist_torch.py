import time
import logging

try:
    import torch
    import torch.nn as nn
    import torch.nn.functional as F
    import torch.optim as optim
    from torchvision import datasets, transforms
    from torch.optim.lr_scheduler import StepLR

except ImportError:
    print("This example requires PyTorch. Please install it to run this script.")
    exit(0)


import emt
from emt import EnergyMonitor
from emt.utils import CSVRecorder, TensorboardRecorder

_NAME = "mnist_example"
logger = logging.getLogger(_NAME)
LOG_FILE_NAME = f"{_NAME}.log"

emt.setup_logger(
    logger,
    log_file_name=LOG_FILE_NAME,
    logging_level=logging.DEBUG,
    mode="w",
)


class Net(nn.Module):
    def __init__(self):
        super(Net, self).__init__()
        self.conv1 = nn.Conv2d(1, 32, 3, 1)
        self.conv2 = nn.Conv2d(32, 64, 3, 1)
        self.dropout1 = nn.Dropout(0.25)
        self.dropout2 = nn.Dropout(0.5)
        self.fc1 = nn.Linear(9216, 128)
        self.fc2 = nn.Linear(128, 10)

    def forward(self, x):
        x = self.conv1(x)
        x = F.relu(x)
        x = self.conv2(x)
        x = F.relu(x)
        x = F.max_pool2d(x, 2)
        x = self.dropout1(x)
        x = torch.flatten(x, 1)
        x = self.fc1(x)
        x = F.relu(x)
        x = self.dropout2(x)
        x = self.fc2(x)
        output = F.log_softmax(x, dim=1)
        return output


class MNISTPipeline:

    def __init__(self, epochs):
        self.batch_size = 64
        self.test_batch_size = 1000
        self.lr = 1.0
        self.gamma = 0.7
        self.epochs = epochs

        self.use_cuda = torch.cuda.is_available()
        self.use_mps = torch.backends.mps.is_available()

        if self.use_cuda:
            self.device = torch.device("cuda")
        elif self.use_mps:
            self.device = torch.device("mps")
        else:
            self.device = torch.device("cpu")

        print("Loading Data ...")
        train_loader, test_loader = self.load_data()

        print("Building Model ...")
        self.model = Net().to(self.device)
        self.optimizer = optim.Adadelta(
            self.model.parameters(), lr=self.lr, weight_decay=0.0
        )
        scheduler = StepLR(self.optimizer, step_size=1, gamma=self.gamma)

        print("Starting Training ...")
        for epoch in range(1, self.epochs + 1):
            self.train(train_loader, epoch)
            self.test(test_loader)
            scheduler.step()

    def load_data(self):
        """Load and preprocess the MNIST dataset."""
        train_kwargs = {"batch_size": self.batch_size}
        test_kwargs = {"batch_size": self.test_batch_size}
        if self.use_cuda:
            cuda_kwargs = {"num_workers": 1, "pin_memory": True, "shuffle": True}
            train_kwargs.update(cuda_kwargs)
            test_kwargs.update(cuda_kwargs)

        transform = transforms.Compose(
            [transforms.ToTensor(), transforms.Normalize((0.1307,), (0.3081,))]
        )
        dataset1 = datasets.MNIST(
            "./data", train=True, download=True, transform=transform
        )
        dataset2 = datasets.MNIST("./data", train=False, transform=transform)
        train_loader = torch.utils.data.DataLoader(dataset1, **train_kwargs)
        test_loader = torch.utils.data.DataLoader(dataset2, **test_kwargs)
        return train_loader, test_loader

    def train(self, train_loader, epoch):
        self.model.train()
        for batch_idx, (data, target) in enumerate(train_loader):
            data, target = data.to(self.device), target.to(self.device)
            self.optimizer.zero_grad()
            output = self.model(data)
            loss = F.nll_loss(output, target)
            loss.backward()
            self.optimizer.step()
        print(
            "\nTrain Epoch: {} [{}/{} ({:.0f}%)]\tLoss: {:.6f}".format(
                epoch,
                batch_idx * len(data),
                len(train_loader.dataset),
                100.0 * batch_idx / len(train_loader),
                loss.item(),
            )
        )

    def test(self, test_loader):
        self.model.eval()
        test_loss = 0
        correct = 0
        with torch.no_grad():
            for data, target in test_loader:
                data, target = data.to(self.device), target.to(self.device)
                output = self.model(data)
                test_loss += F.nll_loss(
                    output, target, reduction="sum"
                ).item()  # sum up batch loss
                pred = output.argmax(
                    dim=1, keepdim=True
                )  # get the index of the max log-probability
                correct += pred.eq(target.view_as(pred)).sum().item()

        test_loss /= len(test_loader.dataset)

        print(
            "Test: Average loss: {:.4f}, Accuracy: {}/{} ({:.0f}%)\n".format(
                test_loss,
                correct,
                len(test_loader.dataset),
                100.0 * correct / len(test_loader.dataset),
            )
        )


def run_mnist_flow(epochs=5):
    with EnergyMonitor(
        name=_NAME,
        trace_recorders=[
            CSVRecorder(write_interval=50),
            TensorboardRecorder(write_interval=10),
        ],
    ) as monitor:
        start_time = time.time()
        MNISTPipeline(epochs=epochs)
        execution_time = time.time() - start_time
        print(f"Execution time: {execution_time:.2f} Seconds.")
        print(f"Energy consumption: {monitor.total_consumed_energy:.2f} J")
        print(f"Energy consumption: {monitor.consumed_energy}")


if __name__ == "__main__":
    run_mnist_flow(epochs=10)
