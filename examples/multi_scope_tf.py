import time
import timeit
import numpy as np
import logging


try:
    import tensorflow as tf
    from tensorflow.keras import layers, models
    from tensorflow.keras.datasets import mnist

except ImportError:
    print("This example requires TensorFlow. Please install it to run this script.")
    exit(0)

import emt
from emt import EnergyMonitor
from emt.utils import CSVRecorder, TensorboardRecorder


_NAME = "multi_scope_tf"
logger = logging.getLogger(_NAME)
LOG_FILE_NAME = f"{_NAME}.log"

emt.setup_logger(
    logger,
    log_file_name=LOG_FILE_NAME,
    logging_level=logging.DEBUG,
    mode="w",
)


def add_tensors_gpu(device="gpu"):
    with tf.device(device):
        # Generate random data
        a = tf.random.uniform(shape=(1000,), minval=1, maxval=100, dtype=tf.int32)
        b = tf.random.uniform(shape=(1000,), minval=1, maxval=100, dtype=tf.int32)
        return a + b


# This implementation ensures that MNIST runs only on gpu 0
class MNIST:
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
            verbose=1,
        )

    def evaluate_model(self, x_test, y_test):
        """Evaluate the model on test data."""
        test_loss, test_accuracy = self.model.evaluate(x_test, y_test)
        print(f"Test accuracy: {test_accuracy:.2f}, Test Loss: {test_loss:.2f}")


def run_training(epochs=20):
    """Run the MNIST training and evaluation."""
    with EnergyMonitor(
        name="tf_addition",
        trace_recorders=[
            CSVRecorder(write_interval=50),
            TensorboardRecorder(write_interval=10),
        ],
    ) as monitor_tf_add:
        start_time_1 = time.time()
        # repeat the addition 10000 times
        _ = timeit.timeit(add_tensors_gpu, number=30000)

        with EnergyMonitor(
            name="tf_mnist",
            trace_recorders=[
                CSVRecorder(write_interval=50),
                TensorboardRecorder(write_interval=10),
            ],
        ) as monitor_mnist:

            start_time = time.time()
            MNIST(epochs=epochs)
            execution_time = time.time() - start_time
            print(f"\n\nMNIST execution time: {execution_time:.2f} Seconds.")
            print(
                f"MNIST energy consumption: {monitor_mnist.total_consumed_energy:.2f} J"
            )
            print(f"MNIST energy consumption: {monitor_mnist.consumed_energy}")

        execution_time_1 = time.time() - start_time_1

    print(f"\n\nTF + MNIST execution time: {execution_time_1:.2f} Seconds.")
    print(
        f"TF + MNIST energy consumption: {monitor_tf_add.total_consumed_energy:.2f} J"
    )
    print(f"TF + MNIST energy consumption: {monitor_tf_add.consumed_energy}")


if __name__ == "__main__":
    run_training(epochs=10)
    print("Done!")
