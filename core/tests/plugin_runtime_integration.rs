use std::sync::{Arc, Mutex};
use mainstage_core::vm::{VM, plugin::Plugin, value::Value};
use async_trait::async_trait;

type CalledArgs = Vec<(String, Vec<Value>)>;

struct TestPlugin {
    name: String,
    called: Arc<Mutex<CalledArgs>>,
}

impl TestPlugin {
    fn new(name: &str, called: Arc<Mutex<CalledArgs>>) -> Self {
        Self { name: name.to_string(), called }
    }
}

#[async_trait]
impl Plugin for TestPlugin {
    fn name(&self) -> &str { &self.name }

    async fn call(&self, func: &str, args: Vec<Value>) -> Result<Value, String> {
        let mut lock = self.called.lock().unwrap();
        lock.push((func.to_string(), args));
        Ok(Value::Int(123))
    }

    fn metadata(&self) -> mainstage_core::vm::plugin::PluginMetadata { mainstage_core::vm::plugin::PluginMetadata::default() }
    fn as_any(&self) -> &dyn std::any::Any { self }
}

fn write_u32_le(buf: &mut Vec<u8>, v: u32) {
    buf.extend(&v.to_le_bytes());
}
fn write_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    write_u32_le(buf, bytes.len() as u32);
    buf.extend(bytes);
}

#[test]
fn plugin_call_end_to_end() {
    // Build a tiny bytecode image with a single PluginCall op followed by Halt
    // Header: "MSBC", version 1, op_count 2
    let mut bytes: Vec<u8> = Vec::new();
    bytes.extend(b"MSBC");
    write_u32_le(&mut bytes, 1); // version
    write_u32_le(&mut bytes, 2); // op count

    // Op 0: PluginCall (0x72)
    bytes.push(0x72);
    write_string(&mut bytes, "test_plugin"); // plugin_name
    write_string(&mut bytes, "echo"); // func_name
    write_u32_le(&mut bytes, 0); // argc
    write_u32_le(&mut bytes, 1); // has_result
    write_u32_le(&mut bytes, 0); // result reg

    // Op 1: Halt (0x50)
    bytes.push(0x50);

    // Prepare VM and register in-process plugin
    let called = Arc::new(Mutex::new(Vec::new()));
    let plugin = TestPlugin::new("test_plugin", called.clone());
    let mut vm = VM::new(bytes);
    vm.register_plugin(Arc::new(plugin));

    // Run VM
    let res = vm.run(false);
    assert!(res.is_ok(), "VM run failed: {:?}", res.err());

    // Verify plugin was invoked
    let lock = called.lock().unwrap();
    assert_eq!(lock.len(), 1, "plugin should have been called once");
    assert_eq!(lock[0].0, "echo");
    assert!(lock[0].1.is_empty(), "no args expected");
}
