#!/usr/bin/env python3
from __future__ import annotations

import argparse
import os
from pathlib import Path

if "MPLCONFIGDIR" not in os.environ:
    os.environ["MPLCONFIGDIR"] = str(Path(__file__).resolve().parent.parent / ".mplconfig")

import matplotlib

matplotlib.use("Agg")
import matplotlib.image as mpimg
import matplotlib.pyplot as plt

PANEL_SPECS = [
    ("KLL", Path("plots/kll/kll_accuracy_relative_error.png")),
    ("CMS", Path("plots/cms/cms_accuracy_avg_relative_error_from_8192.png")),
    ("CS", Path("plots/cs/cs_accuracy_avg_relative_error_from_8192.png")),
    ("HLL", Path("plots/hll/hll_accuracy_relative_error.png")),
    ("DD", Path("plots/dd/dd_accuracy_relative_error.png")),
    ("Nitro", Path("plots/nitro/nitro_accuracy_relative_error.png")),
    ("Octo", Path("plots/octo/octo_accuracy_cms.png")),
]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    accuracy_root = Path(__file__).resolve().parent.parent
    images = []
    for title, relative_path in PANEL_SPECS:
        image_path = accuracy_root / relative_path
        if not image_path.exists():
            raise FileNotFoundError(f"required source plot is missing: {image_path}")
        images.append((title, mpimg.imread(image_path)))

    args.output.parent.mkdir(parents=True, exist_ok=True)
    fig, axes = plt.subplots(4, 2, figsize=(20, 24), constrained_layout=True)
    flat_axes = list(axes.flat)

    for ax, (title, image) in zip(flat_axes, images):
        ax.imshow(image)
        ax.set_title(title)
        ax.axis("off")

    for ax in flat_axes[len(images):]:
        ax.axis("off")

    fig.savefig(args.output, dpi=200)
    plt.close(fig)


if __name__ == "__main__":
    main()
