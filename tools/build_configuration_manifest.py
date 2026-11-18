#!/usr/bin/env python3
"""Pin the composed resource catalogues to the current architecture profile."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FOLDER = ROOT/'docs/inventory/LFE5U-85F'
CATALOGUES = ['decode_catalog.json', 'iologic_research/config_catalogue.json',
              'pio_bank_research/config_catalogue.json', 'clock_system_research/config_catalogue.json',
              'dqs_research/config_catalogue.json']


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    model = FOLDER/'model_manifest.json'
    report = {'schema_version': 1, 'device': 'LFE5U-85F', 'architecture_manifest_sha256': sha(model),
              'files': [{'path': n, 'sha256': sha(FOLDER/n)} for n in CATALOGUES],
              'generator_sha256': sha(Path(__file__))}
    (FOLDER/'configuration_manifest.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps({'catalogues': len(CATALOGUES), 'architecture_manifest_sha256': report['architecture_manifest_sha256']}))


if __name__ == '__main__':
    main()
