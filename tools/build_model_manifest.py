#!/usr/bin/env python3
"""Pin the researched architecture inputs used by the Rust DeviceModel loader."""
import json
from pathlib import Path
from inventory_trellis import sha256

ROOT = Path(__file__).resolve().parents[1]
FOLDER = ROOT / 'docs/inventory/LFE5U-85F'
INPUTS = [
    'device_inventory.json', 'resource_instances.csv', 'site_bindings.csv',
    'resource_ports.csv', 'iologic_research/resource_ports.csv',
    'plc_research/node_templates.csv', 'plc_research/native_ports.csv',
    'package_system_research/physical_pins.csv',
]


def main():
    inventory = json.loads((FOLDER / 'device_inventory.json').read_text())
    db = Path(inventory['database_root']).expanduser()
    for source in inventory['source_files']:
        if sha256(db / source['path']) != source['sha256']:
            raise ValueError('stale database evidence: ' + source['path'])
    report = {
        'schema_version': 1, 'identity_schema_version': 1,
        'device': inventory['device'],
        'database_revision': inventory['database_revision'],
        'trellis_revision': inventory['trellis_revision'],
        'scope': 'architecture_inputs;not_a_behavioural_model',
        'files': [{'path': name, 'sha256': sha256(FOLDER / name)} for name in INPUTS],
        'generator_sha256': sha256(Path(__file__)),
    }
    (FOLDER / 'model_manifest.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'device': report['device'], 'pinned_files': len(report['files'])}))


if __name__ == '__main__':
    main()
