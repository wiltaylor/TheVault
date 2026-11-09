import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';

// Exercise the production downloader without network access or retry sleeps.
const setup = `
import importlib.util, pathlib, tempfile, hashlib
from unittest.mock import patch
spec = importlib.util.spec_from_file_location('wrapper', 'scripts/release/gradle-wrapper.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
tmp = tempfile.TemporaryDirectory()
root = pathlib.Path(tmp.name)
properties = root / 'gradle-wrapper.properties'
original = 'distributionUrl=https\\\\://services.gradle.org/distributions/gradle-9.6.1-bin.zip\\n'
properties.write_text(original)
cache = root / 'cache'
payload = b'verified distribution'
checksum = hashlib.sha256(payload).hexdigest()
calls = []
class Response:
    def __init__(self, data):
        import io
        self.data = io.BytesIO(data)
    def __enter__(self): return self.data
    def __exit__(self, *args): self.data.close()
def fetch(url, **kwargs):
    calls.append(url)
    if url.endswith('.sha256'): return Response(checksum.encode())
    if 'services.gradle.org' in url and fallback: raise OSError('504')
    return Response(b'corrupt' if corrupt else payload)
`;

for (const [name, body] of [
  ['primary download is verified', `fallback=False; corrupt=False
with patch.object(m.urllib.request, 'urlopen', fetch): m.prepare(properties, cache)
assert len(calls) == 2
assert 'distributionUrl=file\\\\:' in properties.read_text()
assert 'distributionSha256Sum=' + checksum in properties.read_text()`],
  ['504 retries then uses GitHub fallback', `fallback=True; corrupt=False
with patch.object(m.urllib.request, 'urlopen', fetch), patch.object(m.time, 'sleep'): m.prepare(properties, cache)
assert len(calls) == 5
assert calls[-1] == 'https://github.com/gradle/gradle-distributions/releases/download/v9.6.1/gradle-9.6.1-bin.zip'
assert (cache / 'gradle-9.6.1-bin.zip').read_bytes() == payload`],
  ['bad checksum fails without changing wrapper', `fallback=True; corrupt=True
with patch.object(m.urllib.request, 'urlopen', fetch), patch.object(m.time, 'sleep'):
    try: m.prepare(properties, cache)
    except ValueError as error: assert 'mismatch' in str(error)
    else: raise AssertionError('accepted bad checksum')
assert properties.read_text() == original
assert not list(cache.iterdir())`],
  ['cache hit is verified against official checksum', `fallback=False; corrupt=False
cache.mkdir()
(cache / 'gradle-9.6.1-bin.zip').write_bytes(payload)
with patch.object(m.urllib.request, 'urlopen', fetch): m.prepare(properties, cache)
assert len(calls) == 1`],
  ['corrupt cache is replaced', `fallback=False; corrupt=False
cache.mkdir()
(cache / 'gradle-9.6.1-bin.zip').write_bytes(b'bad cache')
with patch.object(m.urllib.request, 'urlopen', fetch): m.prepare(properties, cache)
assert len(calls) == 2
assert (cache / 'gradle-9.6.1-bin.zip').read_bytes() == payload`],
]) {
  test(name, () => {
    assert.doesNotThrow(() => execFileSync('python3', ['-c', setup + '\n' + body], { stdio: 'pipe' }));
  });
}
