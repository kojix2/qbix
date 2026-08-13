#!/usr/bin/env python3
"""Generate monochrome figures for the qbix paper."""

import csv
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt


ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "data"
FIGURES = ROOT / "figures"
TIME_FLOOR = 0.005

STYLES = {
    "qbix query order": ("o-", "0.05"),
    "qbix BAM order": ("^:", "0.25"),
    "Atlantool": ("s--", "0.45"),
    "samtools full scan": ("D-.", "0.65"),
}


def main() -> None:
    with (DATA / "query-time-present.csv").open(newline="") as handle:
        rows = list(csv.DictReader(handle))

    plt.rcParams.update(
        {
            "font.family": "DejaVu Sans",
            "font.size": 8.5,
            "axes.spines.top": False,
            "axes.spines.right": False,
        }
    )
    datasets = ["Illumina", "PacBio HiFi", "Oxford Nanopore"]
    figure, axes = plt.subplots(1, 3, figsize=(7.2, 2.65), sharey=True)

    for panel, (axis, dataset) in enumerate(zip(axes, datasets), start=1):
        for method, (style, color) in STYLES.items():
            values = [
                row for row in rows
                if row["dataset"] == dataset and row["method"] == method
            ]
            values.sort(key=lambda row: int(row["qnames"]))
            x = [int(row["qnames"]) for row in values]
            y = [max(float(row["median_s"]), TIME_FLOOR) for row in values]
            axis.plot(
                x,
                y,
                style,
                color=color,
                linewidth=1.4,
                markersize=4,
                markerfacecolor="white" if method == "qbix BAM order" else color,
                markeredgecolor=color,
                label=method,
            )
        axis.set_xscale("log")
        axis.set_yscale("log")
        axis.set_title(f"{chr(64 + panel)}. {dataset}")
        axis.set_xlabel("Queried QNAMEs")
        axis.set_xticks([1, 10, 100, 1_000, 10_000])
        axis.grid(axis="y", which="major", color="0.88", linewidth=0.7)

    axes[0].set_ylabel("Median wall time (s)")
    handles, labels = axes[0].get_legend_handles_labels()
    figure.legend(
        handles,
        labels,
        loc="lower center",
        ncol=4,
        frameon=False,
        fontsize=7.5,
        bbox_to_anchor=(0.5, -0.01),
    )
    figure.tight_layout(rect=(0, 0.12, 1, 1), pad=0.6)
    FIGURES.mkdir(exist_ok=True)
    figure.savefig(
        FIGURES / "query-scaling.png",
        dpi=240,
        bbox_inches="tight",
        facecolor="white",
    )
    plt.close(figure)


if __name__ == "__main__":
    main()
