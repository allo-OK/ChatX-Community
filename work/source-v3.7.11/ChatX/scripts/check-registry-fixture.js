'use strict';
const fs=require('fs'),path=require('path'),{randomUUID}=require('crypto'),{spawnSync}=require('child_process');
module.exports=async function(compiler,root,temp){
 const escape=s=>s.replace(/\$/g,()=> '$$').replace(/"/g,'$\\"');
 const key='Software\\ChatXInstallerTests\\'+randomUUID();
 const output=path.join(temp,'registry-test.exe'),report=path.join(temp,'registry.ini'),script=path.join(temp,'registry.nsi');
 fs.writeFileSync(script,`
!include "MUI2.nsh"
!define KC_LEGACY_UNINSTALL_KEY "${key}"
!define BUILD_RESOURCES_DIR "${escape(path.join(root,'build'))}"
!include "${escape(path.join(root,'build/upgrade.nsh'))}"
Name "ChatX registry regression fixture"
OutFile "${escape(output)}"
RequestExecutionLevel user
SilentInstall silent
Section
 SetRegView 32
 WriteRegStr HKCU "${key}" "Fixture" "1"
 Call kcRemoveLegacyPerUser
 WriteINIStr "${escape(report)}" "result" "empty" "PASS"
 WriteRegStr HKCU "${key}\\Other" "DisplayName" "Unrelated fixture"
 Call kcRemoveLegacyPerUser
 WriteINIStr "${escape(report)}" "result" "unrelated" "PASS"
 WriteRegStr HKCU "${key}\\Broken" "DisplayName" "ChatX Chat"
 WriteRegStr HKCU "${key}\\Broken" "UninstallString" ""
 Call kcRemoveLegacyPerUser
 WriteINIStr "${escape(report)}" "result" "invalid" "PASS"
 DeleteRegKey HKCU "${key}"
 SetErrorLevel 0
SectionEnd
`);
 const built=spawnSync(compiler.path,['-V2',script],{env:{...process.env,...compiler.env},encoding:'utf8',timeout:60000});
 if(built.error||built.status!==0)throw Error(built.error?.message||built.stderr||built.stdout);
 console.log('PASS registry termination fixture compilation');
 if(process.platform!=='win32'){console.log('SKIP registry runtime: Windows required');return;}
 try{
  const run=spawnSync(output,['/S'],{timeout:20000,windowsHide:true});
  if(run.error||run.status!==0)throw Error('Registry enumeration hangs or fails: '+(run.error?.message||run.status));
  const raw=fs.readFileSync(report),text=raw.includes(0)?raw.toString('utf16le'):raw.toString();
  for(const field of ['empty','unrelated','invalid'])if(!text.includes(field+'=PASS'))throw Error('Missing registry result '+field);
  console.log('PASS registry runtime: empty, unrelated and invalid legacy entries');
 }finally{spawnSync('reg.exe',['delete','HKCU\\'+key,'/f','/reg:32'],{timeout:5000,windowsHide:true});}
};
