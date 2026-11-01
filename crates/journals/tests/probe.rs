// Exercise the unregistered probe's response lifecycle in the ordinary test
// lane, so mutation filters select the same tests as explicit example runs.
#[path = "../examples/journal-persist.rs"]
#[allow(dead_code)]
mod probe;
