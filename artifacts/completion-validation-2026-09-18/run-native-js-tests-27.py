import sys, os, json, time, subprocess
from pathlib import Path
root=Path(__file__).resolve().parents[2]
os.chdir(root)
sys.path.insert(0,str(root/'scripts'))
from generation_gate import GenerationLease
base=root/'artifacts/completion-validation-2026-09-18'
try:
 lease=GenerationLease(root/'docs/oryn2/GENERATION-COMPLETE-REPAIR-41-2026-09-18.json')
 env=lease.environment()
except Exception as error:
 result=dict(exit_code=1,phase='generation_admission',generation_error=str(error))
 (base/'native-js-tests-27.json').write_text(json.dumps(result,indent=2)+'\n')
 print(json.dumps(result),flush=True)
 sys.exit(1)
for key in ('V8_FROM_SOURCE','ORYN_STARTUP_SNAPSHOT','ORYN_ICU_DATA_PATH'):env.pop(key,None)
env.update(SDKROOT='/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.5.sdk',
RUSTY_V8_ARCHIVE=str(root/'artifacts/v8-schema5-2026-09-18/librusty_v8-core-full-7e4604e068025681bd0788e1720a948576363f581ac55c7d46d11e7b14de00ef.a'),
RUSTY_V8_SRC_BINDING_PATH=str(root/'artifacts/v8-schema5-2026-09-18/src_binding-core-full-2eb9b14accc74085e6bdbdfa7e039464454286f70e044d19771492af0074db28.rs'),
ORYN_V8_BRIDGE_ABI=str(root/'artifacts/v8-schema5-2026-09-18/v8-bridge-abi-core-full.json'),
ORYN_ICU_DATA_FINGERPRINT_PATH=str(root/'target/rusty-v8-src-150.4.0-schema5/third_party/icu/common/icudtl.dat'),
RUST_BACKTRACE='1')
cmd=['cargo','test','--locked','--no-fail-fast','-p','oryn-native','-p','oryn-js','-p','oryn-dom','-p','oryn-webidl','-p','oryn-layout','-p','oryn-common','-p','oryn-net','-p','oryn-render-core','--features','oryn-native/v8-host,oryn-native/in-process-probe,oryn-native/v8-sandbox,oryn-native/core-full','--','--nocapture']
(base/'native-js-tests-27.preflight.json').write_text(json.dumps(dict(command=cmd,declaration_sha256=lease.sha256,bridge_abi=env['ORYN_V8_BRIDGE_ABI']),indent=2)+'\n')
check=['cargo','run','--locked','--offline','-p','oryn-webidl','--','check','crates/oryn-webidl/idl/browser.webidl','crates/oryn-js/src/generated/browser.rs']
attempt_start=time.monotonic()
with (base/'webidl-check-27.log').open('w') as log:
 check_code=subprocess.call(check,env=env,stdout=log,stderr=subprocess.STDOUT)
(base/'webidl-check-27.json').write_text(json.dumps(dict(exit_code=check_code,declaration_sha256=lease.sha256,command=check),indent=2)+'\n')
try:lease.check()
except Exception as error:
 result=dict(exit_code=check_code or 1,phase='registry_check',elapsed_seconds=time.monotonic()-attempt_start,declaration_sha256=lease.sha256,generation_error=str(error),log='artifacts/completion-validation-2026-09-18/webidl-check-27.log')
 (base/'native-js-tests-27.json').write_text(json.dumps(result,indent=2)+'\n')
 print(json.dumps(result),flush=True)
 sys.exit(result['exit_code'])
if check_code:
 result=dict(exit_code=check_code,phase='registry_check',elapsed_seconds=time.monotonic()-attempt_start,declaration_sha256=lease.sha256,log='artifacts/completion-validation-2026-09-18/webidl-check-27.log')
 (base/'native-js-tests-27.json').write_text(json.dumps(result,indent=2)+'\n')
 print(json.dumps(result),flush=True)
 sys.exit(check_code)
start=time.monotonic()
with (base/'native-js-tests-27.log').open('w') as log:
 code=subprocess.call(cmd,env=env,stdout=log,stderr=subprocess.STDOUT)
result=dict(exit_code=code,phase='tests',elapsed_seconds=time.monotonic()-start,declaration_sha256=lease.sha256,log='artifacts/completion-validation-2026-09-18/native-js-tests-27.log')
try:lease.check()
except Exception as error:result['generation_error']=str(error)
(base/'native-js-tests-27.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result),flush=True)
sys.exit(code or bool(result.get('generation_error')))
