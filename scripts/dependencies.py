#!/usr/bin/env python3
"""Inventory the locked dependency graph and actual license files for private use.

This is an evidence inventory, not an automatic interpretation of legal terms.
"""
import hashlib
import json
import pathlib
import subprocess
ROOT=pathlib.Path(__file__).resolve().parents[1]

def main():
    result=subprocess.run([str(ROOT/'scripts/cargo.sh'),'metadata','--locked','--format-version','1'],cwd=ROOT,capture_output=True,text=True,check=True)
    metadata=json.loads(result.stdout)
    nodes={n['id']:n for n in metadata['resolve']['nodes']}
    active={}
    for target in ['aarch64-apple-darwin','x86_64-unknown-linux-gnu','aarch64-unknown-linux-gnu']:
        filtered=json.loads(subprocess.check_output([str(ROOT/'scripts/cargo.sh'),'metadata','--locked','--format-version','1','--filter-platform',target],cwd=ROOT))
        graph={node['id']:node['dependencies'] for node in filtered['resolve']['nodes']}
        seen=set();pending=[filtered['resolve']['root']]
        while pending:
            node=pending.pop()
            if node in seen:continue
            seen.add(node);pending.extend(graph.get(node,[]))
        for node in seen:active.setdefault(node,[]).append(target)
    packages=[]
    for package in metadata['packages']:
        if package['name']=='architect':continue
        directory=pathlib.Path(package['manifest_path']).parent
        licenses=[]
        for path in sorted(directory.rglob('*')):
            if path.is_file() and path.name.upper().startswith(('LICENSE','COPYING','NOTICE','UNLICENSE')):
                licenses.append(dict(path=str(path.relative_to(directory)),sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
        packages.append(dict(name=package['name'],version=package['version'],source=package['source'],
                             declared_expression=package['license'],license_files=licenses,active_targets=active.get(package['id'],[]),
                             features=nodes.get(package['id'],{}).get('features',[]),
                             review='source files inventoried' if licenses else 'requires upstream license source review'))
    lock_hash=hashlib.sha256((ROOT/'Cargo.lock').read_bytes()).hexdigest()
    report=dict(schema_version=1,distribution='private local use; no public code or binary distribution authorized',
                lock_sha256=lock_hash,scope='complete Cargo metadata graph including target-specific/dev packages; see features per node',
                limitations=['No vulnerability database scan is claimed.', 'License expressions are preserved verbatim; no AND/OR/WITH expression is silently rewritten.',
                             'Reassess obligations before any distribution, dependency/features change, or vendoring.'],packages=packages)
    (ROOT/'docs/dependencies.json').write_text(json.dumps(report,indent=2)+'\n')
    print(f"{len(packages)} locked packages inventoried; {sum(not p['license_files'] and bool(p['active_targets']) for p in packages)} active packages lack license source files")

if __name__=='__main__':main()
