use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmExecutionResult {
    pub success: bool,
    pub gas_used: u64,
    pub logs: Vec<String>,
    pub storage_mutations: HashMap<String, String>,
    pub error: Option<String>,
}

pub fn execute_bytecode(
    caller: &str,
    contract: &str,
    bytecode_hex: &str,
    state_tree: &sled::Tree,
) -> VmExecutionResult {
    let mut gas_used: u64 = 21000;
    let mut stack: Vec<i64> = Vec::with_capacity(1024);
    let mut logs = Vec::new();
    let mut storage_mutations = HashMap::new();

    let clean_code = bytecode_hex.trim_start_matches("0x");
    let tokens: Vec<&str> = clean_code.split_whitespace().collect();
    let mut pc = 0;

    while pc < tokens.len() {
        gas_used += 100;
        if gas_used > 5_000_000 {
            return VmExecutionResult {
                success: false,
                gas_used,
                logs,
                storage_mutations: HashMap::new(),
                error: Some("Execution halted: CoreByte Out of Gas limit (5,000,000)".to_string()),
            };
        }

        if stack.len() > 1024 {
            return VmExecutionResult {
                success: false,
                gas_used,
                logs,
                storage_mutations: HashMap::new(),
                error: Some("Execution halted: Stack Overflow Guard (Max Depth 1024)".to_string()),
            };
        }

        match tokens[pc] {
            "PUSH" => {
                pc += 1;
                if pc < tokens.len() {
                    let val: i64 = tokens[pc].parse().unwrap_or(0);
                    stack.push(val);
                }
            }
            "ADD" => {
                if stack.len() >= 2 {
                    let b = stack.pop().unwrap();
                    let a = stack.pop().unwrap();
                    stack.push(a.saturating_add(b));
                }
            }
            "SUB" => {
                if stack.len() >= 2 {
                    let b = stack.pop().unwrap();
                    let a = stack.pop().unwrap();
                    stack.push(a.saturating_sub(b));
                }
            }
            "MUL" => {
                if stack.len() >= 2 {
                    let b = stack.pop().unwrap();
                    let a = stack.pop().unwrap();
                    stack.push(a.saturating_mul(b));
                    gas_used += 50;
                }
            }
            "DIV" => {
                if stack.len() >= 2 {
                    let b = stack.pop().unwrap();
                    let a = stack.pop().unwrap();
                    if b != 0 {
                        stack.push(a / b);
                    } else {
                        stack.push(0);
                    }
                    gas_used += 50;
                }
            }
            "EQ" => {
                if stack.len() >= 2 {
                    let b = stack.pop().unwrap();
                    let a = stack.pop().unwrap();
                    stack.push(if a == b { 1 } else { 0 });
                }
            }
            "SSTORE" => {
                pc += 1;
                if pc < tokens.len() && !stack.is_empty() {
                    let key = format!("CB_CONTRACT:{}:{}", contract, tokens[pc]);
                    let val = stack.pop().unwrap().to_string();
                    storage_mutations.insert(key, val);
                    gas_used += 5000;
                }
            }
            "SLOAD" => {
                pc += 1;
                if pc < tokens.len() {
                    let key = format!("CB_CONTRACT:{}:{}", contract, tokens[pc]);
                    let val = storage_mutations.get(&key).cloned().or_else(|| {
                        state_tree.get(key.as_bytes()).ok().flatten().and_then(|v| String::from_utf8(v.to_vec()).ok())
                    }).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
                    stack.push(val);
                    gas_used += 800;
                }
            }
            "LOG" => {
                pc += 1;
                if pc < tokens.len() {
                    logs.push(format!("[CoreByte-VM LOG from {}]: {}", caller, tokens[pc]));
                    gas_used += 375;
                }
            }
            _ => {}
        }
        pc += 1;
    }

    VmExecutionResult {
        success: true,
        gas_used,
        logs,
        storage_mutations,
        error: None,
    }
}