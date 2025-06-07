import time
from tqdm import tqdm
import logging
import torch
from torch.utils.data import DataLoader
from torch.utils.tensorboard import SummaryWriter
from torch.optim import AdamW
from transformers import BertTokenizer, BertForSequenceClassification
from datasets import load_dataset, DatasetDict
from sklearn.metrics import accuracy_score

import emt
from emt import EnergyMonitor
from emt.utils import TensorboardRecorder

_NAME = "sentiment_analysis"
logger = logging.getLogger(_NAME)
LOG_FILE_NAME = f"{_NAME}.log"

emt.setup_logger(
    logger,
    log_file_name=LOG_FILE_NAME,
    logging_level=logging.DEBUG,
    mode="w",
)

# initialize the general summary writer
LOG_TF_EVENTS_PATH = f"./tf_logs/{_NAME}/"
summary_writer = SummaryWriter(
    LOG_TF_EVENTS_PATH,
)

# Set device (GPU if available, else CPU)
DEVICE = torch.device("cuda" if torch.cuda.is_available() else "cpu")
# Constants
MAX_LENGTH = 128
BATCH_SIZE = 32
LEARNING_RATE = 2e-5
NUM_CLASSES = 2


class BERTModel(torch.nn.Module):
    def __init__(self):
        super(BERTModel, self).__init__()
        self.bert = BertForSequenceClassification.from_pretrained(
            "bert-base-uncased", num_labels=NUM_CLASSES
        )

    def forward(self, input_ids, attention_mask):
        outputs = self.bert(input_ids=input_ids, attention_mask=attention_mask)
        return outputs.logits


class SentimentPipeline:
    def __init__(self):
        self.reduce_frac = 0.3

    # 1. Data Loading and Preprocessing
    def load_and_preprocess_data(self):
        """Load and preprocess the IMDb dataset."""
        dataset = load_dataset("imdb")

        # Filter out 'unsupervised' and keep only 'train' and 'test'
        dataset = DatasetDict({key: dataset[key] for key in ["train", "test"]})

        # Take only 30% of train and test datasets
        dataset["train"] = dataset["train"].select(
            range(int(len(dataset["train"]) * self.reduce_frac))
        )
        dataset["test"] = dataset["test"].select(
            range(int(len(dataset["test"]) * self.reduce_frac))
        )
        tokenizer = BertTokenizer.from_pretrained("bert-base-uncased")

        def tokenize_function(examples):
            return tokenizer(
                examples["text"],
                padding="max_length",
                truncation=True,
                max_length=MAX_LENGTH,
            )

        tokenized_datasets = dataset.map(tokenize_function, batched=True)
        tokenized_datasets.set_format(
            type="torch", columns=["input_ids", "attention_mask", "label"]
        )

        train_loader = DataLoader(
            tokenized_datasets["train"],
            batch_size=BATCH_SIZE,
            shuffle=True,
            num_workers=4,
        )
        test_loader = DataLoader(
            tokenized_datasets["test"], batch_size=BATCH_SIZE, num_workers=4
        )

        return train_loader, test_loader

    def train_step(self, model, train_loader):
        """Train the model."""
        optimizer = AdamW(
            model.parameters(),
            lr=LEARNING_RATE,
            weight_decay=0.01,
        )
        criterion = torch.nn.CrossEntropyLoss()
        model.train()
        total_loss = 0

        # Add progress bar
        progress_bar = tqdm(train_loader, desc="Training", leave=False)

        for batch in progress_bar:
            input_ids = batch["input_ids"].to(DEVICE)
            attention_mask = batch["attention_mask"].to(DEVICE)
            labels = batch["label"].to(DEVICE)

            optimizer.zero_grad()
            outputs = model(input_ids, attention_mask)
            loss = criterion(outputs, labels)
            loss.backward()
            optimizer.step()

            total_loss += loss.item()

            # Update progress bar
            progress_bar.set_postfix({"Loss": loss.item()})

        avg_loss = total_loss / len(train_loader)
        print(f"Training Loss: {avg_loss:.4f}")
        return avg_loss

    def evaluate_step(self, model, test_loader):
        """Evaluate the model."""
        model.eval()
        predictions, true_labels = [], []

        # Add progress bar
        progress_bar = tqdm(test_loader, desc="Evaluating", leave=False)

        with torch.no_grad():
            for batch in progress_bar:
                input_ids = batch["input_ids"].to(DEVICE)
                attention_mask = batch["attention_mask"].to(DEVICE)
                labels = batch["label"].to(DEVICE)

                outputs = model(input_ids, attention_mask)
                _, predicted_labels = torch.max(outputs, dim=1)

                predictions.extend(predicted_labels.cpu().numpy())
                true_labels.extend(labels.cpu().numpy())

        accuracy = accuracy_score(true_labels, predictions)
        print(f"Accuracy: {accuracy:.4f}")
        return accuracy

    def run(self, epochs: int = 3):
        """Run the sentiment analysis pipeline."""

        print("Getting dataset and creating data loaders...")
        train_loader, test_loader = self.load_and_preprocess_data()

        print("Initializing BERT model...")
        model = BERTModel().to(DEVICE)

        print("Starting training and evaluation epochs...")
        for epoch in tqdm(range(epochs)):
            print(f"Epoch {epoch + 1}")
            train_loss = self.train_step(model, train_loader)
            test_acc = self.evaluate_step(model, test_loader)
            # use the global writer to log the metrics
            summary_writer.add_scalar("Loss/train", train_loss, epoch)
            summary_writer.add_scalar("Accuracy/test", test_acc, epoch)
            summary_writer.flush()


if __name__ == "__main__":

    with EnergyMonitor(
        name=_NAME,
        # pass existing general writer to TensorboardRecorder
        trace_recorders=[TensorboardRecorder(writer=summary_writer)],
    ) as monitor:
        start_time = time.time()
        pipeline = SentimentPipeline()
        pipeline.run(epochs=2)

        execution_time = time.time() - start_time
        print(f"execution time: {execution_time:.2f} Seconds.")
        print(f"energy consumption: {monitor.total_consumed_energy:.2f} J")
        print(f"energy consumption: {monitor.consumed_energy}")
