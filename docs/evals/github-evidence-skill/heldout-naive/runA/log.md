## Invocation 1
```
$ casegraphen github project --capture-dir capture --manifest manifest.json --require-independent-review --strict
(exit 1)
```

## Invocation 2
```
$ casegraphen github project --capture-dir capture --manifest manifest.json --require-independent-review --strict --format json
(exit 1)
{"data":{"findings":[{"code":"artifact_path_escape","detail":"capture/issue-92.json","location":"$.entries[].artifact_path"}]},"error_code":"invalid","message":"contract validation failed: [{\"code\":\"artifact_path_escape\",\"location\":\"$.entries[].artifact_path\",\"detail\":\"capture/issue-92.json\"}]","refusal_version":1,"schema":"highergraphen.case.native_cli.refusal.v1"}
```

## Invocation 3
```
$ casegraphen github project --capture-dir . --manifest manifest.json --require-independent-review --strict --format json
(exit 2)
{"input":{"command":"casegraphen github project"},"metadata":{"command":"casegraphen github project","core_packages":["higher-graphen-core"],"tool_package":"casegraphen"},"projection":{"ai_view":{"native_boundary":"CaseSpace plus MorphismLog state is replayed before derived reports are emitted.","operation":"casegraphen github project"},"audit_trace":{"information_loss":["Native CLI operation reports include the operation result but not a full command-line argv transcript."],"source_ids":[]},"human_review":{"summary":"Native CaseGraphen CLI operation completed."}},"report_type":"native_cli_operation","report_version":1,"result":{"accepted":false,"domain_findings":[{"code":"projection_blocking_finding","detail":"require_independent_review is set and no independent human approval is bound to the observed head","location":"$.projection.blocking_findings[independent_review_policy:github-observation:CAPHTECH/casegraphen#101@c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b]"}],"independence":{"classifications":[{"basis":"check_observation","evidence_role":"ci_check","subject_id":"check:c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b:check_run:quality:a7b991b7d530238c67cc9ad353061dffcee7e0321e09bc0ba25ad8c381bbfc69"},{"basis":"check_observation","evidence_role":"ci_check","subject_id":"check:c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b:check_run:quality:b2fb950e42945b9f64345f3418a0e83cead60586b80dc1aac533f1ce70d582ec"},{"basis":"check_observation","evidence_role":"ci_check","subject_id":"check:c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b:status_context:CodeRabbit:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"},{"basis":"provider_bot_discriminator","evidence_role":"automated_bot","subject_id":"finding:054db4a2452aa9d06d99211c539d24b4dc8a24dd76b95f87cb82cbdcf3b62d7a"},{"basis":"author_in_implementation_actor_set","evidence_role":"self_review","subject_id":"finding:0b8f0cfe342ef35495175ed9add89dd0a43b6fe95ac2140155f09b6ac24e66f5"},{"basis":"author_in_implementation_actor_set","evidence_role":"self_review","subject_id":"finding:0e72a9672c53593c8c97284c71a0f65b3c928304ca438c511a3dba1c46ad844d"},{"basis":"provider_bot_discriminator","evidence_role":"automated_bot","subject_id":"finding:13b9bc6b6524aff94b6837c93e9ef540a535a59bcdec89150be84d8163b42a66"},{"basis":"provider_bot_discriminator","evidence_role":"automated_bot","subject_id":"finding:2a12217584111f98934b0b71cfd1b1f66846e6111f493097aae5351812862079"},{"basis":"author_in_implementation_actor_set","evidence_role":"self_review","subject_id":"finding:2f1cdd98e5846edbf5dc7dc71038b9d7a56392e385ecd0f47ce570a8175b2b22"},{"basis":"author_in_implementation_actor_set","evidence_role":"self_review","subject_id":"finding:2fe293489d1715f286e79af62cf283ab0dbb0b176122abce8dc863fb420e81f3"},{"basis":"author_in_implementation_actor_set","evidence_role":"self_review","subject_id":"finding:396c40b128403af0e02db6dc4b6259c0f36fceaff9d66a565304eee67417b597"},{"basis":"provider_bot_discriminator","evidence_role":"automated_bot","subject_id":"finding:3c6a30ea302f6032292503d1cf1f780f18694ce85dfee7bf5e6c4aa03df37d37"},{"basis":"provider_bot_discriminator","evidence_role":"automated_bot","subject_id":"finding:4000dd8d895a280c65d117f316676cf53e9218b32dd4122eb688c1c5c78b6c4a"},{"basis":"provider_bot_discriminator","evidence_role":"automated_bot","subject_id":"finding:44cff42807677106d1fda3f7123d72d3fd24e33641de5e22fa6a9be899136df8"},{"basis":"author_in_implementation_actor_set","evidence_role":"self_review","subject_id":"finding:4cb0f1d19a5b8249ef02dfd982d7b5c7b9c1d49377c6c2478159d3cb48bbb091"},{"basis":"author_in_implementation_actor_set","evidence_role":"self_review","subject_id":"finding:4e74bc1790767d9f846dad58714cc42c35d6642ff809c0b554fcac918667f00b"},{"basis":"provider_bot_discriminator","evidence_role":"automated_bot","subject_id":"finding:52b6d632a78ee2b95140e111d38b1a8f2e51a95275c95b15e4b36c107bcf36b1"},{"basis":"provider_bot_discriminator","evidence_role":"automated_bot","subject_id":"finding:5b3e65ded051c
```

## Invocation 4
```
$ casegraphen github project --capture-dir . --manifest manifest.json --require-independent-review --format json
(exit 0)
{
 "input": {
  "command": "casegraphen github project"
 },
 "metadata": {
  "command": "casegraphen github project",
  "core_packages": [
   "higher-graphen-core"
  ],
  "tool_package": "casegraphen"
 },
 "projection": {
  "ai_view": {
   "native_boundary": "CaseSpace plus MorphismLog state is replayed before derived reports are emitted.",
   "operation": "casegraphen github project"
  },
  "audit_trace": {
   "information_loss": [
    "Native CLI operation reports include the operation result but not a full command-line argv transcript."
   ],
   "source_ids": []
  },
  "human_review": {
   "summary": "Native CaseGraphen CLI operation completed."
  }
 },
 "report_type": "native_cli_operation",
 "report_version": 1,
 "result": {
  "accepted": false,
  "domain_findings": [
   {
    "code": "projection_blocking_finding",
    "detail": "require_independent_review is set and no independent human approval is bound to the observed head",
    "location": "$.projection.blocking_findings[independent_review_policy:github-observation:CAPHTECH/casegraphen#101@c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b]"
   }
  ],
  "mutation_performed": false,
  "projection": {
   "accepted": false,
   "base_sha": "947f347f219a60775bcf71b226ce778cc8ea21f4",
   "blocking_findings": [
    {
     "finding_id": "independent_review_policy:github-observation:CAPHTECH/casegraphen#101@c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b",
     "reason": "require_independent_review is set and no independent human approval is bound to the observed head"
    }
   ],
   "can_skim": [
    {
     "path": "README.md",
     "reason": "no review findings recorded against this file",
     "subject_ids": []
    },
    {
     "path": "docs/adr/0002-graph-engineering-positioning.md",
     "reason": "no review findings recorded against this file",
     "subject_ids": []
    },
    {
     "path": "docs/adr/0012-adr-identifier-inventory.md",
     "reason": "no review findings recorded against this file",
     "subject_ids": []
    },
    {
     "path": "docs/adr/0020-graph-engineering-product-surface.md",
     "reason": "no review findings recorded against this file",
     "subject_ids": []
    },
    {
     "path": "docs/adr/0028-memory-plane-positioning.md",
     "reason": "no review findings recorded against this file",
     "subject_ids": []
    },
    {
     "path": "docs/adr/0029-memory-plane-stable-promotion.md",
     "reason": "no review findings recorded against this file",
     "subject_ids": []
    },
    {
     "path": "docs/design/memory-plane.md",
     "reason": "no review findings recorded against this file",
     "subject_ids": []
    },
    {
     "path": "docs/design/memory-threat-model.md",
     "reason": "no review findings recorded against this file",
     "subject_ids": []
    },
    {
     "path": "docs/guides/mcp-operational-host.md",
     "reason": "no review findings recorded against this file",
     "subject_ids": []
    },
    {
     "path": "docs/guides/memory-plane.md",
     "reaso
```

## Invocation 5 (ci-gate.sh proof)
```
$ ./ci-gate.sh
(exit 2)
```

## Invocation 6
```
$ casegraphen github refresh --capture-dir . --manifest manifest2.json --previous-capture-dir . --previous-manifest manifest.json --format json
(exit 0)
{"input":{"command":"casegraphen github refresh"},"metadata":{"command":"casegraphen github refresh","core_packages":["higher-graphen-core"],"tool_package":"casegraphen"},"projection":{"ai_view":{"native_boundary":"CaseSpace plus MorphismLog state is replayed before derived reports are emitted.","operation":"casegraphen github refresh"},"audit_trace":{"information_loss":["Native CLI operation reports include the operation result but not a full command-line argv transcript."],"source_ids":[]},"human_review":{"summary":"Native CaseGraphen CLI operation completed."}},"report_type":"native_cli_operation","report_version":1,"result":{"accepted":false,"domain_findings":[{"code":"stale_head","detail":"the observed head aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa no longer matches the previous review basis's head c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b; a refresh never rebases — run `github observe` on the new capture instead","location":"$.refresh_result.disposition"}],"mutation_performed":false,"refresh_result":{"disposition":"stale_head","observation_changes":[],"observed_base_sha":"947f347f219a60775bcf71b226ce778cc8ea21f4","observed_head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","previous_base_sha":"947f347f219a60775bcf71b226ce778cc8ea21f4","previous_head_sha":"c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b","previous_observation_hash":"sha256:748c176398e9481901f1ecaa07396faab02441faca2aa610586cc70bc2f76004","review_basis_moved":false,"schema":"casegraphen.experimental.github.refresh_result.v0"}},"schema":"highergraphen.case.native_cli.report.v1"}
```

## Invocation 7
```
$ casegraphen github refresh --capture-dir . --manifest manifest2.json --previous-capture-dir . --previous-manifest manifest.json --format json --strict
(exit 2)
{"input":{"command":"casegraphen github refresh"},"metadata":{"command":"casegraphen github refresh","core_packages":["higher-graphen-core"],"tool_package":"casegraphen"},"projection":{"ai_view":{"native_boundary":"CaseSpace plus MorphismLog state is replayed before derived reports are emitted.","operation":"casegraphen github refresh"},"audit_trace":{"information_loss":["Native CLI operation reports include the operation result but not a full command-line argv transcript."],"source_ids":[]},"human_review":{"summary":"Native CaseGraphen CLI operation completed."}},"report_type":"native_cli_operation","report_version":1,"result":{"accepted":false,"domain_findings":[{"code":"stale_head","detail":"the observed head aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa no longer matches the previous review basis's head c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b; a refresh never rebases — run `github observe` on the new capture instead","location":"$.refresh_result.disposition"}],"mutation_performed":false,"refresh_result":{"disposition":"stale_head","observation_changes":[],"observed_base_sha":"947f347f219a60775bcf71b226ce778cc8ea21f4","observed_head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","previous_base_sha":"947f347f219a60775bcf71b226ce778cc8ea21f4","previous_head_sha":"c9be9ed6ac51e2b9aeadb2906b990f1b168ee41b","previous_observation_hash":"sha256:748c176398e9481901f1ecaa07396faab02441faca2aa610586cc70bc2f76004","review_basis_moved":false,"schema":"casegraphen.experimental.github.refresh_result.v0"}},"schema":"highergraphen.case.native_cli.report.v1"}
```

