// This file is part of Substrate.

// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use criterion::{criterion_group, criterion_main, Criterion, BenchmarkGroup};
use criterion::measurement::WallTime;

use codec::Encode;

use sc_executor_common::{
	runtime_blob::RuntimeBlob,
	wasm_runtime::{WasmInstance, WasmModule, DEFAULT_HEAP_ALLOC_STRATEGY},
};
use sc_executor_wasmtime::InstantiationStrategy;
use sc_runtime_test::wasm_binary_unwrap as test_runtime;
use std::sync::{
	atomic::{AtomicBool, AtomicUsize, Ordering},
	Arc,
};

#[derive(Clone)]
enum Method {
	Compiled { instantiation_strategy: InstantiationStrategy, precompile: bool },
}

// This is just a bog-standard Kusama runtime with an extra
// `test_empty_return` and `test_dirty_plenty_memory` functions
// copy-pasted from the test runtime.
fn kusama_runtime() -> &'static [u8] {
	include_bytes!("kusama_runtime.wasm")
}

fn initialize(
	_tmpdir: &mut Option<tempfile::TempDir>,
	runtime: &[u8],
	method: Method,
) -> Box<dyn WasmModule> {
	let blob = RuntimeBlob::uncompress_if_needed(runtime).unwrap();

	let allow_missing_func_imports = true;

	match method {
		Method::Compiled { instantiation_strategy, precompile } => {
			let config = sc_executor_wasmtime::Config {
				allow_missing_func_imports,
				cache_path: None,
				semantics: sc_executor_wasmtime::Semantics {
					heap_alloc_strategy: DEFAULT_HEAP_ALLOC_STRATEGY,
					instantiation_strategy,
					deterministic_stack_limit: None,
					canonicalize_nans: false,
					parallel_compilation: true,
					wasm_multi_value: false,
					wasm_bulk_memory: false,
					wasm_reference_types: false,
					wasm_simd: false,
				},
			};

			if precompile {
				let precompiled_blob =
					sc_executor_wasmtime::prepare_runtime_artifact(blob, &config.semantics)
						.unwrap();

				// Create a fresh temporary directory to make absolutely sure
				// we'll use the right module.
				*_tmpdir = Some(tempfile::tempdir().unwrap());
				let tmpdir = _tmpdir.as_ref().unwrap();

				let path = tmpdir.path().join("module.bin");
				std::fs::write(&path, &precompiled_blob).unwrap();
				unsafe {
					sc_executor_wasmtime::create_runtime_from_artifact::<
						sp_io::SubstrateHostFunctions,
					>(&path, config)
				}
			} else {
				sc_executor_wasmtime::create_runtime::<sp_io::SubstrateHostFunctions>(blob, config)
			}
			.map(|runtime| -> Box<dyn WasmModule> { Box::new(runtime) })
		},
	}
	.unwrap()
}

fn run_benchmark(
	c: &mut Criterion,
	benchmark_name: &str,
	thread_count: usize,
	runtime: &dyn WasmModule,
	testcase: impl Fn(&mut Box<dyn WasmInstance>) + Copy + Send + 'static,
) {
	c.bench_function(benchmark_name, |b| {
		// Here we deliberately start a bunch of extra threads which will just
		// keep on independently instantiating the runtime over and over again.
		//
		// We don't really have to measure how much time those take since the
		// work done is essentially the same on each thread, and what we're
		// interested in here is only how those extra threads affect the execution
		// on the current thread.
		//
		// In an ideal case assuming we have enough CPU cores those extra threads
		// shouldn't affect the main thread's runtime at all, however in practice
		// they're not completely independent. There might be per-process
		// locks in the kernel which are briefly held during instantiation, etc.,
		// and how much those affect the execution here is what we want to measure.
		let is_benchmark_running = Arc::new(AtomicBool::new(true));
		let threads_running = Arc::new(AtomicUsize::new(0));
		let aux_threads: Vec<_> = (0..thread_count - 1)
			.map(|_| {
				let mut instance = runtime.new_instance().unwrap();
				let is_benchmark_running = is_benchmark_running.clone();
				let threads_running = threads_running.clone();
				std::thread::spawn(move || {
					threads_running.fetch_add(1, Ordering::SeqCst);
					while is_benchmark_running.load(Ordering::Relaxed) {
						testcase(&mut instance);
					}
				})
			})
			.collect();

		while threads_running.load(Ordering::SeqCst) != (thread_count - 1) {
			std::thread::yield_now();
		}

		let mut instance = runtime.new_instance().unwrap();
		b.iter(|| testcase(&mut instance));

		is_benchmark_running.store(false, Ordering::SeqCst);
		for thread in aux_threads {
			thread.join().unwrap();
		}
	});
}

fn bench_call_instance(c: &mut Criterion) {
	sp_tracing::try_init_simple();

	let strategies = [
		(
			"recreate_instance_vanilla",
			Method::Compiled {
				instantiation_strategy: InstantiationStrategy::RecreateInstance,
				precompile: false,
			},
		),
		(
			"recreate_instance_cow_fresh",
			Method::Compiled {
				instantiation_strategy: InstantiationStrategy::RecreateInstanceCopyOnWrite,
				precompile: false,
			},
		),
		(
			"recreate_instance_cow_precompiled",
			Method::Compiled {
				instantiation_strategy: InstantiationStrategy::RecreateInstanceCopyOnWrite,
				precompile: true,
			},
		),
		(
			"pooling_vanilla_fresh",
			Method::Compiled {
				instantiation_strategy: InstantiationStrategy::Pooling,
				precompile: false,
			},
		),
		(
			"pooling_vanilla_precompiled",
			Method::Compiled {
				instantiation_strategy: InstantiationStrategy::Pooling,
				precompile: true,
			},
		),
		(
			"pooling_cow_fresh",
			Method::Compiled {
				instantiation_strategy: InstantiationStrategy::PoolingCopyOnWrite,
				precompile: false,
			},
		),
		(
			"pooling_cow_precompiled",
			Method::Compiled {
				instantiation_strategy: InstantiationStrategy::PoolingCopyOnWrite,
				precompile: true,
			},
		),
	];

	let runtimes = [("test_runtime", test_runtime())];

	// let thread_counts = [1, 2, 4, 8];
	let thread_counts = [1];

	fn test_call_loop_cpu_function(instance: &mut Box<dyn WasmInstance>) {
		instance.call_export("test_loop_cpu", &[0]).unwrap();
	}

	fn test_call_loop_gpu_function(instance: &mut Box<dyn WasmInstance>) {
		instance.call_export("test_loop_gpu", &[0]).unwrap();
	}

	let testcases = [
		("call_loop_cpu_function", test_call_loop_cpu_function as fn(&mut Box<dyn WasmInstance>)),
		("call_loop_gpu_function", test_call_loop_gpu_function as fn(&mut Box<dyn WasmInstance>)),
	];

	let num_cpus = num_cpus::get_physical();
	let mut tmpdir = None;

	for (strategy_name, strategy) in strategies {
		for (runtime_name, runtime) in runtimes {
			let runtime = initialize(&mut tmpdir, runtime, strategy.clone());
			let mut group: BenchmarkGroup<WallTime> = c.benchmark_group(format!("{strategy_name}_CPU vs GPU"));

			group.bench_function(format!("{strategy_name}_cpu_loop"), |b| {
				let mut instance = runtime.new_instance().unwrap();
				let testcase = testcases[0].1;
				b.iter(|| testcase(&mut instance));
			});

			group.bench_function(format!("{strategy_name}_gpu_loop"), |b| {
				let mut instance = runtime.new_instance().unwrap();
				let testcase = testcases[1].1;
				b.iter(|| testcase(&mut instance));
			});

			group.finish();
		}
	}
}

criterion_group! {
	name = benches_gpu;
	config = Criterion::default();
	targets = bench_call_instance
}
criterion_main!(benches_gpu);
