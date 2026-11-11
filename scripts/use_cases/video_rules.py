#!/usr/bin/env python3

from rules import (above, above_band, above_decayed_band, at_least, both, either, for_consecutive, latch,
                   outside_band)

DIMENSIONS = ("city", "isp", "cdn", "device")
HISTORY, SIGMAS = 30, 3
MUX_REBUFFERING, MUX_START_SECONDS, MUX_VIEWERS = 0.05, 4.0, 100
CONVIVA_HISTORY, CONVIVA_PLAYS, CONVIVA_SUPPRESSION = 360, 25, 60
PATENT_DECAY, PATENT_MINUTES, PATENT_SESSIONS = 0.998, 5, 100


def subpopulation(labels):
    return ".".join(labels[d] for d in DIMENSIONS)


def hydra(v):
    return above_band(v["buffering_l1"], HISTORY, SIGMAS)


def hydra_entropy(v):
    return outside_band(v["entropy_bitrate"], HISTORY, SIGMAS)


def mux(v):
    worse = either(above(v["rebuffer_ratio"], MUX_REBUFFERING), above(v["startup_p95"], MUX_START_SECONDS))
    return both(worse, at_least(v["sessions"], MUX_VIEWERS))


def conviva_ai(v):
    return both(above_band(v["startup_p95"], CONVIVA_HISTORY, SIGMAS, HISTORY), at_least(v["sessions"], CONVIVA_PLAYS))


def conviva_patent(v):
    return both(above_decayed_band(v["rebuffer_ratio"], PATENT_DECAY, SIGMAS, HISTORY),
                at_least(v["distinct_devices"], PATENT_SESSIONS))


RULES = {
    "hydra": {"queries": ("buffering_l1",), "condition": hydra, "entity": subpopulation,
              "description": "Buffering L1 norm above the mean plus 3 standard deviations of the previous "
                             "30 minutes (Hydra)"},
    "hydra_entropy": {"queries": ("entropy_bitrate",), "condition": hydra_entropy, "entity": subpopulation,
                      "description": "Bitrate entropy more than 3 standard deviations from the mean of the "
                                     "previous 30 minutes (Hydra)"},
    "mux": {"queries": ("rebuffer_ratio", "startup_p95", "sessions"), "condition": mux, "entity": subpopulation,
            "description": "Rebuffering above 5% or p95 start time above 4 s over 5 minutes, with at least "
                           "100 viewers (Mux)"},
    "conviva_ai": {"queries": ("startup_p95", "sessions"), "condition": conviva_ai, "entity": subpopulation,
                   "hold": latch(CONVIVA_SUPPRESSION, extend=False),
                   "description": "p95 start time above the mean plus 3 standard deviations of up to 6 hours, "
                                  "with at least 25 plays; the alert then holds for 1 hour (Conviva AI Alerts)"},
    "conviva_patent": {"queries": ("rebuffer_ratio", "distinct_devices"), "condition": conviva_patent,
                       "entity": subpopulation, "hold": for_consecutive(PATENT_MINUTES),
                       "description": "Rebuffering above a baseline decaying by 0.998 per minute plus 3 standard "
                                      "deviations for 5 minutes, with at least 100 devices (Conviva patent)"},
}
