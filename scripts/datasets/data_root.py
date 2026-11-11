"""Where recorded datasets live: --data-root, else $ASAP_DATASETS, else datasets/ in the repository."""

import os
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ENV = "ASAP_DATASETS"


def default():
    return os.environ.get(ENV) or os.path.join(REPO, "datasets")


def existing(path, root):
    if not os.path.exists(path):
        sys.exit(f"{os.path.relpath(path, root)} not found under data root {root} "
                 f"(set it with --data-root or {ENV})")
    return path
