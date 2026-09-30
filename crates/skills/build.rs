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

use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

fn collect(root: &Path, directory: &Path, prefix: &str, files: &mut BTreeMap<String, PathBuf>) {
    println!("cargo:rerun-if-changed={}", directory.display());
    let mut entries = fs::read_dir(directory)
        .expect("bundled skill directory")
        .map(|entry| entry.expect("bundled skill entry"))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type().expect("bundled skill file type");
        assert!(
            !kind.is_symlink(),
            "bundled skills contain only regular files/directories"
        );
        if kind.is_dir() {
            collect(root, &path, prefix, files);
        } else {
            assert!(kind.is_file());
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            if relative == "SKILL.md" {
                continue;
            }
            let name = format!("{prefix}{relative}");
            assert!(
                files.insert(name, path).is_none(),
                "duplicate bundled resource"
            );
        }
    }
}

fn main() {
    let crate_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repo = crate_dir.parent().unwrap().parent().unwrap();
    let mut source = String::from("const BUNDLED: &[Bundled] = &[\n");
    for (id, relative) in [
        ("maka-cua", "crates/computer-use/skills/maka-cua"),
        ("maka-plugin-authoring", "skills/maka-plugin-authoring"),
    ] {
        let root = repo.join(relative);
        let mut files = BTreeMap::new();
        collect(&root, &root, "", &mut files);
        if id == "maka-plugin-authoring" {
            let sdk = repo.join("packages/plugin-sdk/src");
            collect(&sdk, &sdk, "references/sdk/", &mut files);
        }
        let document = root.join("SKILL.md");
        println!("cargo:rerun-if-changed={}", document.display());
        source.push_str(&format!(
            "Bundled {{ id: {id:?}, content: include_str!({:?}), files: &[\n",
            document.to_str().unwrap()
        ));
        for (name, path) in files {
            println!("cargo:rerun-if-changed={}", path.display());
            source.push_str(&format!(
                "({name:?}, include_bytes!({:?})),\n",
                path.to_str().unwrap()
            ));
        }
        source.push_str("] },\n");
    }
    source.push_str("];\n");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("bundled.rs"),
        source,
    )
    .unwrap();
}
