/*
 * Licensed to the Apache Software Foundation (ASF) under one
 * or more contributor license agreements.  See the NOTICE file
 * distributed with this work for additional information
 * regarding copyright ownership.  The ASF licenses this file
 * to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance
 * with the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing,
 * software distributed under the License is distributed on an
 * "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
 * KIND, either express or implied.  See the License for the
 * specific language governing permissions and limitations
 * under the License.
 */

use super::*;
use maka_js_runtime::plugin::Lifecycle;
use std::sync::Arc;

struct NoCalls;
impl Bridge for NoCalls {
    fn call(
        &self,
        method: String,
        _: Value,
    ) -> futures_util::future::BoxFuture<'static, Result<Value, Error>> {
        panic!("registration fixture unexpectedly called Host {method}")
    }
}

#[tokio::test]
async fn sdk_declarations_above_old_inventory_limits_keep_real_result_budget_and_dispose_all_callbacks()
 {
    tokio::time::timeout(Duration::from_secs(20), async {
        let vm = Vm::new(Limits::default()).unwrap();
        let module = vm.load_plugin("many.mjs".into(), r#"
export default async function activate(ctx) {
  for (let index = 0; index < 384; index++) {
    await ctx.remote.method(`entry-${index}`, () => index);
  }
  ctx.effect(() => { globalThis.__makaRegistrationDisposed = true; });
}
"#.into(), Arc::new(NoCalls)).unwrap();
        let declarations = module.call(vec!["activate".into()], vec![json!({}), Value::Null]).await.unwrap();
        assert_eq!(declarations.as_array().unwrap().len(), 384);
        let last = &declarations[383];
        assert_eq!(last["name"], "entry-383");
        let value = module.call(vec!["invoke".into()], vec![last["callback"].clone(),Value::Null,json!({})]).await.unwrap();
        assert_eq!(value["value"],383);
        module.lifecycle(Lifecycle::Retire).await.unwrap();
        module.lifecycle(Lifecycle::Dispose).await.unwrap();
        module.close().await.unwrap();
        let proof = vm.load("proof.mjs".into(), "export function read() { const proof = globalThis.__makaRegistrationDisposed; delete globalThis.__makaRegistrationDisposed; return proof; }".into()).unwrap();
        assert_eq!(call(&proof,"read").await,json!(true));
        proof.close().await.unwrap();
        let large = vm.load_plugin("large.mjs".into(), r#"
export default async function activate(ctx) {
  for (let index = 0; index < 32; index++) {
    await ctx.tui.app(`page-${index}`, {entry:'ui.mjs', backend:()=>null}, {
      title:{fallback:'Page'}, context:'application', commands:[{
        name:'open', title:{fallback:'Open'}, description:{fallback:'Open page'}, route:'x'.repeat(40000)
      }]
    });
  }
}
"#.into(), Arc::new(NoCalls)).unwrap();
        let error = large.call(vec!["activate".into()], vec![json!({}), Value::Null]).await.unwrap_err();
        assert!(error.to_string().contains("result exceeds 1 MiB"),"{error}");
        large.lifecycle(Lifecycle::Retire).await.unwrap();
        large.lifecycle(Lifecycle::Dispose).await.unwrap();
        large.close().await.unwrap();
        let stats = vm.statistics(true).await.unwrap();
        assert_eq!(stats.modules,0);
        assert_eq!(stats.pending_calls,0);
        assert!(!vm.is_terminated());
        vm.shutdown().await;
    }).await.unwrap();
}
