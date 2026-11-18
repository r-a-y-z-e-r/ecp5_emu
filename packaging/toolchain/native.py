"""Discover suite-only native closures; never satisfy a dependency from the host."""
from pathlib import Path
import os
import re
import shutil
import subprocess


def output(*args):
    return subprocess.check_output([str(a) for a in args], text=True)


def native(path, platform):
    with path.open('rb') as stream:
        magic = stream.read(4)
    if platform == 'linux':
        return magic == b'\x7fELF'
    return magic in (b'\xcf\xfa\xed\xfe', b'\xce\xfa\xed\xfe',
                     b'\xfe\xed\xfa\xcf', b'\xca\xfe\xba\xbe', b'\xbe\xba\xfe\xca')


def mac_dependencies(path):
    lines = output('otool', '-L', path).splitlines()[1:]
    ids = output('otool', '-D', path).splitlines()[1:]
    return [line.strip().split(' (compatibility')[0] for line in lines
            if line.strip().split(' (compatibility')[0] not in ids]


def rpaths(path):
    return re.findall(r'cmd LC_RPATH\n\s+cmdsize \d+\n\s+path (.*?) \(offset',
                      output('otool', '-l', path))


def system(path):
    return path.startswith(('/System/Library/', '/usr/lib/'))


class Closure:
    def __init__(self, suite, destination, platform):
        self.suite = suite.resolve()
        self.destination = destination
        self.platform = platform
        self.pending = []
        self.seen = set()
        self.edges = {}
        self.origins = {}

    def copy(self, source):
        """Keep layout and relative aliases, including framework version symlinks."""
        source = Path(os.path.abspath(source))
        relative = source.relative_to(self.suite)
        target = self.destination / relative
        if not source.resolve().is_relative_to(self.suite):
            raise RuntimeError(f'suite link escapes input: {relative}')
        if source.is_symlink():
            resolved = source.resolve(strict=True)
            self.copy(resolved)
            if target.is_symlink():
                target.unlink()
            target.parent.mkdir(parents=True, exist_ok=True)
            target.symlink_to(os.path.relpath(
                self.destination / resolved.relative_to(self.suite), target.parent))
            self.origins[str(relative)] = {'root': 'suite', 'path': str(relative), 'kind': 'symlink'}
            return target
        if source.is_dir():
            target.mkdir(parents=True, exist_ok=True)
            for child in sorted(source.iterdir()):
                self.copy(child)
        else:
            if not target.exists():
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, target)
            self.origins[str(relative)] = {'root': 'suite', 'path': str(relative), 'kind': 'file'}
            if native(source, self.platform) and source not in self.seen:
                self.seen.add(source)
                self.pending.append(source)
        return target

    def resolve(self, name, source):
        if self.platform == 'linux':
            candidates = [self.suite / 'lib' / name, source.parent / name]
        else:
            executable = self.suite / 'libexec/yosys'

            def expand(value):
                return value.replace('@loader_path', str(source.parent)).replace(
                    '@executable_path', str(executable.parent))

            if name.startswith('@rpath/'):
                search = rpaths(source) + rpaths(executable)
                search += rpaths(self.suite / 'libexec/nextpnr-ecp5')
                candidates = [Path(expand(p)) / name[len('@rpath/'):] for p in search]
            else:
                candidates = [Path(expand(name))]
        for candidate in candidates:
            if candidate.is_file() and candidate.resolve().is_relative_to(self.suite):
                return Path(os.path.abspath(candidate))
        raise RuntimeError(f'missing suite library {name} required by {source.relative_to(self.suite)}')

    def collect(self):
        index = 0
        while index < len(self.pending):
            source = self.pending[index]
            index += 1
            if self.platform == 'macos':
                names = [n for n in mac_dependencies(source) if not system(n)]
            else:
                names = re.findall(r'\(NEEDED\).*?\[(.*?)\]', output('readelf', '-d', source))
            edges = []
            for name in names:
                dependency = self.resolve(name, source)
                # Framework metadata/resources and version links accompany its binary.
                framework = next((p for p in dependency.parents if p.suffix == '.framework'), None)
                self.copy(framework if framework else dependency)
                edges.append((name, dependency.resolve()))
            self.edges[source] = edges
        if self.platform == 'macos':
            for source, edges in self.edges.items():
                target = self.destination / source.relative_to(self.suite)
                target.chmod(target.stat().st_mode | 0o200)
                for name, dependency in edges:
                    new = '@loader_path/' + os.path.relpath(
                        self.destination / dependency.relative_to(self.suite), target.parent)
                    subprocess.run(['install_name_tool', '-change', name, new, str(target)], check=True)
                if output('otool', '-D', source).splitlines()[1:]:
                    subprocess.run(['install_name_tool', '-id', '@rpath/' + target.name,
                                    str(target)], check=True)
                for path in set(rpaths(target)):
                    subprocess.run(['install_name_tool', '-delete_rpath', path, str(target)], check=True)
            self.audit_mac()
        print(f'{self.platform}: {len(self.seen)} native files in suite closure')

    def audit_mac(self):
        for source in self.seen:
            target = self.destination / source.relative_to(self.suite)
            for name in mac_dependencies(target):
                if system(name):
                    continue
                if not name.startswith('@loader_path/'):
                    raise RuntimeError(f'external dependency: {target}: {name}')
                resolved = (target.parent / name[len('@loader_path/'):]).resolve(strict=True)
                if not resolved.is_relative_to(self.destination):
                    raise RuntimeError(f'dependency escapes package: {target}: {name}')


def sign(root):
    """Sign libraries/plugins, framework bundles, tool executables, then Zig."""
    files = [p for p in root.rglob('*') if p.is_file() and not p.is_symlink()
             and native(p, 'macos')]
    executables = [p for p in files if p.is_relative_to(root / 'libexec') or p.name == 'zig']
    for path in sorted(set(files) - set(executables), key=lambda p: (-len(p.parts), str(p))):
        subprocess.run(['codesign', '--force', '-s', '-', str(path)], check=True)
    frameworks = [p for p in root.rglob('*.framework') if not p.is_symlink()]
    for path in sorted(frameworks, key=lambda p: (-len(p.parts), str(p))):
        subprocess.run(['codesign', '--force', '-s', '-', str(path)], check=True)
    for path in sorted(executables, key=lambda p: (p.name == 'zig', str(p))):
        subprocess.run(['codesign', '--force', '-s', '-', str(path)], check=True)
