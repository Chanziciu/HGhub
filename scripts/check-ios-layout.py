"""Run the actual native app and compare window, WebView, and DOM heights."""
import json
import os
from pathlib import Path
import subprocess
import time

def run(*args, **kwargs):
    print('Running:', ' '.join(map(str, args)), flush=True)
    return subprocess.check_output(args, text=True, timeout=120, **kwargs).strip()

out = Path('layout-evidence')
out.mkdir(exist_ok=True)
app = Path('apps.txt').read_text().splitlines()[0]
runtimes = json.loads(run('xcrun', 'simctl', 'list', 'runtimes', '-j'))['runtimes']
runtime = next(r['identifier'] for r in reversed(runtimes) if r.get('isAvailable') and 'iOS' in r['name'])
types = json.loads(run('xcrun', 'simctl', 'list', 'devicetypes', '-j'))['devicetypes']
for model in ['iPhone 11 Pro Max', 'iPhone 16 Pro Max']:
    # 11 Pro Max has the same 414x896 point screen as XS Max, and supports current runtimes.
    device_type = next(t['identifier'] for t in types if t['name'] == model)
    device = run('xcrun', 'simctl', 'create', 'HGhub '+model, device_type, runtime)
    try:
        run('xcrun', 'simctl', 'boot', device)
        run('xcrun', 'simctl', 'bootstatus', device, '-b')
        run('xcrun', 'simctl', 'install', device, app)
        env = dict(os.environ, SIMCTL_CHILD_HGHUB_PROXY='http://127.0.0.1:1')
        run('xcrun', 'simctl', 'launch', device, 'com.hghub.huangguo', env=env)
        time.sleep(10)
        container = Path(run('xcrun', 'simctl', 'get_app_container', device, 'com.hghub.huangguo', 'data'))
        logs = list(container.rglob('debug.log'))
        assert logs, 'No native layout log'
        text = logs[0].read_text()
        (out/(model+'.log')).write_text(text)
        run('xcrun', 'simctl', 'io', device, 'screenshot', str(out/(model+'.png')))
        measurements = [json.loads(line.split('[layout] ', 1)[1]) for line in text.splitlines() if '[layout] {' in line]
        assert measurements, text
        m = measurements[-1]
        print(model, json.dumps(m), flush=True)
        assert abs(m['height']-m['windowHeight']) < 1, 'WebView does not fill window'
        assert abs(m['rootHeight']-m['windowHeight']) < 1, 'DOM does not fill window'
        assert abs(m['windowHeight']-m['screenHeight']) < 1, 'Window does not fill phone screen'
        # Exercise a background/foreground transition as well as first launch.
        run('xcrun', 'simctl', 'launch', device, 'com.apple.Preferences')
        run('xcrun', 'simctl', 'launch', device, 'com.hghub.huangguo', env=env)
        time.sleep(3)
        run('xcrun', 'simctl', 'io', device, 'screenshot', str(out/(model+'-resumed.png')))
    finally:
        subprocess.run(['xcrun','simctl','shutdown',device], check=False, timeout=60)
