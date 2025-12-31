Introduces Hierarchical span names: `service.lifecycle` → `service.launch`

```
2025-12-29T09:23:50.544397Z INFO service.lifecycle: strata_service::sync_worker: service starting service.name=chain_worker service.name=chain_worker service.type=sync
2025-12-29T09:23:50.544453Z INFO service.lifecycle:service.launch: strata_chain_worker::service: waiting until genesis service.name=chain_worker service.type=sync service.name=chain_worker
2025-12-29T09:23:50.544518Z INFO service.lifecycle:service.launch: strata_chain_worker::service: initializing chain worker blkid=1c7a48..fb7ff3 service.name=chain_worker service.type=sync service.name=chain_worker
2025-12-29T09:23:50.544541Z INFO service.lifecycle:service.launch: strata_service::sync_worker: service launch completed service.name=chain_worker duration_ms=0 service.name=chain_worker service.type=sync
2025-12-29T09:23:50.544559Z INFO service.lifecycle:service.launch: strata_service::sync_worker: close time.busy=104µs time.idle=4.92µs service.name=chain_worker service.type=sync
```

- clear parent child relationships in span name
- Explicit Duration tracking `duration_ms=0`
- Structured attributes: `service.name`, `service.type`
- semantic span names

```
2025-12-29T09:37:51.431130Z  INFO asm.lifecycle: strata_service::sync_worker: service starting service.name=asm_worker
2025-12-29T09:37:51.431177Z  INFO csm.lifecycle: strata_service::sync_worker: service starting service.name=csm_worker
2025-12-29T09:37:51.430953Z  INFO chain.lifecycle: strata_service::sync_worker: service starting service.name=chain_worker

2025-12-29T09:37:51.434277Z  INFO asm.lifecycle:asm.launch: strata_service::sync_worker: service launch completed duration_ms=3
2025-12-29T09:37:51.431307Z  INFO csm.lifecycle:csm.launch: strata_service::sync_worker: service launch completed duration_ms=0
2025-12-29T09:37:51.431166Z  INFO chain.lifecycle:chain.launch: strata_service::sync_worker: service launch completed duration_ms=0

2025-12-29T09:37:51.486759Z  INFO asm.lifecycle:asm.process_message: strata_asm_worker::service: ASM found pivot anchor state
2025-12-29T09:37:51.514384Z  INFO asm.lifecycle:asm.process_message: strata_asm_worker::service: Created genesis manifest leaf_index=0
```


- clear trace hierarchy
```
   asm.lifecycle
     ├── asm.launch
     ├── asm.process_message
     │   ├── asm.transition
     │   └── asm.store_state
     └── asm.shutdown

```
