import importlib
import json
import os
import pkgutil

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
WORKLOADS = os.path.join(REPO, "aqpbm-usecase", "workloads")


def load():
    found = {}
    for info in pkgutil.iter_modules(__path__):
        module = importlib.import_module(f"{__name__}.{info.name}")
        found[module.ID] = module
    return dict(sorted(found.items()))


def spec_of(module):
    with open(os.path.join(WORKLOADS, module.CASE["spec"])) as f:
        return json.load(f)
