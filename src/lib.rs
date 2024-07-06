pub mod managed_thread;

use std::sync::atomic::Ordering::SeqCst;

#[cfg(test)]
use managed_thread::AtomicU32;
#[cfg(not(test))]
use std::sync::atomic::AtomicU32;

#[derive(Default)]
pub struct Counter {
  value: AtomicU32,
}

impl Counter {
  pub fn increment(&self) {
    // self.value.fetch_add(1, SeqCst);
    let value = self.value.load(SeqCst);
    self.value.store(value + 1, SeqCst);
  }

  pub fn get(&self) -> u32 {
    self.value.load(SeqCst)
  }
}

#[test]
fn threaded_test() {
  let counter = Counter::default();

  let thread_count = 100;
  let increment_count = 100;

  std::thread::scope(|scope| {
    for _ in 0..thread_count {
      scope.spawn(|| {
        for _ in 0..increment_count {
          counter.increment()
        }
      });
    }
  });

  assert_eq!(counter.get(), thread_count * increment_count);
}

#[test]
fn pbt() {
  arbtest::arbtest(|rng| {
    eprintln!("begin trace");
    let counter = Counter::default();
    let mut counter_model: u32 = 0;

    std::thread::scope(|scope| {
      let t1 = managed_thread::spawn(scope, &counter);
      let t2 = managed_thread::spawn(scope, &counter);
      let mut threads = [t1, t2];

      while !rng.is_empty() {
        for (tid, t) in threads.iter_mut().enumerate() {
          if rng.arbitrary()? {
            if t.is_paused() {
              eprintln!("{tid}: unpause");
              t.unpause()
            } else {
              eprintln!("{tid}: increment");
              t.submit(|c| c.increment());
              counter_model += 1;
            }
          }
        }
      }

      for t in threads {
        t.join();
      }
      assert_eq!(counter_model, counter.get());

      Ok(())
    })
  })
  .seed(0x9c2a13a600000001);
}

#[test]
fn exhaustytest() {
  let mut g = exhaustigen::Gen::new();
  let mut interleavings_count = 0;

  while !g.done() {
    interleavings_count += 1;
    let counter = Counter::default();
    let mut counter_model: u32 = 0;

    let increment_count = g.gen(5) as u32;
    std::thread::scope(|scope| {
      let t1 = managed_thread::spawn(scope, &counter);
      let t2 = managed_thread::spawn(scope, &counter);

      'outer: while t1.is_paused()
        || t2.is_paused()
        || counter_model < increment_count
      {
        for t in [&t1, &t2] {
          if g.flip() {
            if t.is_paused() {
              t.unpause();
              continue 'outer;
            }
            if counter_model < increment_count {
              t.submit(|c| c.increment());
              counter_model += 1;
              continue 'outer;
            }
          }
        }
        return for t in [t1, t2] {
          t.join()
        };
      }

      assert_eq!(counter_model, counter.get());
    });
  }
  eprintln!("all {interleavings_count} interleavings are fine!");
}
