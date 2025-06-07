import sys
import time
import logging
import numpy as np

try:
    import tensorflow as tf
    from tensorflow.keras import layers, models
    from tensorflow.keras.datasets import mnist
except ImportError:
    print("This example requires TensorFlow. Please install it to run this script.")
    sys.exit(0)


import emt
from emt import EnergyMonitor
from emt.utils import CSVRecorder, TensorboardRecorder

_NAME = "mnist_tf"
logger = logging.getLogger(_NAME)
LOG_FILE_NAME = f"{_NAME}.log"

emt.setup_logger(
    logger,
    log_file_name=LOG_FILE_NAME,
    logging_level=logging.DEBUG,
    mode="w",
)


# This implementation ensures that MNIST runs only on gpu 0
class MNISTPipeline:
    def __init__(self, epochs, batch_size=32):

        self.epochs = epochs
        self.batch_size = batch_size
        print("Loading Data ...")
        (x_train, y_train), (x_test, y_test) = self.load_data()
        print("Building Model ...")
        self.build_model()
        print("Training Started ...")
        self.train_model(
            x_train, y_train, epochs=self.epochs, batch_size=self.batch_size
        )
        self.evaluate_model(x_test, y_test)

    def load_data(
        self,
    ):
        """Load and preprocess the MNIST dataset."""
        (x_train, y_train), (x_test, y_test) = mnist.load_data()

        # Normalize the images to [0, 1]
        x_train = x_train.astype("float32") / 255.0
        x_test = x_test.astype("float32") / 255.0

        # Reshape the data to add a channel dimension
        x_train = np.expand_dims(x_train, -1)
        x_test = np.expand_dims(x_test, -1)

        return (x_train, y_train), (x_test, y_test)

    def build_model(
        self,
    ):
        """Create a CNN model for image classification."""
        self.model = models.Sequential(
            [
                layers.Input(shape=(28, 28, 1)),
                layers.Conv2D(32, (3, 3), activation="relu"),
                layers.MaxPooling2D((2, 2)),
                layers.Conv2D(64, (3, 3), activation="relu"),
                layers.MaxPooling2D((2, 2)),
                layers.Conv2D(64, (3, 3), activation="relu"),
                layers.Flatten(),
                layers.Dense(64, activation="relu"),
                layers.Dense(10, activation="softmax"),  # 10 classes for digits 0-9
            ]
        )

        self.model.compile(
            optimizer="adam",
            loss="sparse_categorical_crossentropy",
            metrics=["accuracy"],
        )

    def train_model(self, x_train, y_train, epochs=5, batch_size=64):
        """Train the CNN model."""
        self.model.fit(
            x_train,
            y_train,
            epochs=epochs,
            batch_size=batch_size,
            validation_split=0.2,
            verbose=0,  # Suppress progress bar
        )

    def evaluate_model(self, x_test, y_test):
        """Evaluate the model on test data."""
        test_loss, test_accuracy = self.model.evaluate(x_test, y_test)
        print(f"Test accuracy: {test_accuracy:.2f}, Test Loss: {test_loss:.2f}")


def run_mnist_flow(epochs=10, batch_size=32):
    """Run the MNIST pipeline."""
    with EnergyMonitor(
        name=_NAME,
        trace_recorders=[
            CSVRecorder(write_interval=60),
            TensorboardRecorder(write_interval=30),
        ],
    ) as monitor:
        start_time = time.time()
        MNISTPipeline(epochs=epochs, batch_size=batch_size)
        execution_time = time.time() - start_time
        print(f"execution time: {execution_time:.2f} Seconds.")
        print(f"energy consumption: {monitor.total_consumed_energy:.2f} J")
        print(f"energy consumption: {monitor.consumed_energy}")


if __name__ == "__main__":
    run_mnist_flow(epochs=10, batch_size=32)
