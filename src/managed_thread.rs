use std::{
  cell::RefCell,
  sync::{atomic::Ordering, mpsc, Arc, Condvar, Mutex},
  thread::Scope,
};

#[derive(Default)]
pub struct AtomicU32 {
  inner: std::sync::atomic::AtomicU32,
}

impl AtomicU32 {
  pub fn load(&self, ordering: Ordering) -> u32 {
    pause();
    let result = self.inner.load(ordering);
    pause();
    result
  }

  pub fn store(&self, value: u32, ordering: Ordering) {
    pause();
    self.inner.store(value, ordering);
    pause();
  }

  pub fn fetch_add(
    &self,
    value: u32,
    ordering: Ordering,
  ) -> u32 {
    pause();
    let result = self.inner.fetch_add(value, ordering);
    pause();
    result
  }
}

fn pause() {
  if let Some(ctx) = SharedContext::get() {
    ctx.pause()
  }
}

#[derive(Default)]
struct SharedContext {
  state: Mutex<State>,
  cv: Condvar,
}

#[derive(Default, PartialEq, Eq, Debug)]
enum State {
  #[default]
  Ready,
  Running,
  Paused,
}

thread_local! {
  static INSTANCE: RefCell<Option<Arc<SharedContext>>> = RefCell::new(None);
}

impl SharedContext {
  fn set(ctx: Arc<SharedContext>) {
    INSTANCE.with(|it| *it.borrow_mut() = Some(ctx));
  }

  fn get() -> Option<Arc<SharedContext>> {
    INSTANCE.with(|it| it.borrow().clone())
  }

  fn pause(&self) {
    let mut guard = self.state.lock().unwrap();
    assert_eq!(*guard, State::Running);
    *guard = State::Paused;
    self.cv.notify_all();
    guard = self
      .cv
      .wait_while(guard, |state| *state == State::Paused)
      .unwrap();
    assert_eq!(*guard, State::Running)
  }
}

pub struct ManagedHandle<'scope, T> {
  inner: std::thread::ScopedJoinHandle<'scope, ()>,
  sender: mpsc::Sender<Box<dyn FnOnce(&mut T) + 'scope + Send>>,
  ctx: Arc<SharedContext>,
}

pub fn spawn<'scope, T: 'scope + Send>(
  scope: &'scope Scope<'scope, '_>,
  mut state: T,
) -> ManagedHandle<'scope, T> {
  let ctx: Arc<SharedContext> = Default::default();
  let (sender, receiver) =
    mpsc::channel::<Box<dyn FnOnce(&mut T) + 'scope + Send>>();
  let inner = scope.spawn({
    let ctx = Arc::clone(&ctx);
    move || {
      SharedContext::set(Arc::clone(&ctx));
      for f in receiver {
        f(&mut state);
        let mut guard = ctx.state.lock().unwrap();
        assert_eq!(*guard, State::Running);
        *guard = State::Ready;
        ctx.cv.notify_all()
      }
    }
  });
  ManagedHandle { inner, ctx, sender }
}

impl<'scope, T> ManagedHandle<'scope, T> {
  pub fn is_paused(&self) -> bool {
    let guard = self.ctx.state.lock().unwrap();
    *guard == State::Paused
  }

  pub fn unpause(&self) {
    let mut guard = self.ctx.state.lock().unwrap();
    assert_eq!(*guard, State::Paused);
    *guard = State::Running;
    self.ctx.cv.notify_all();
    guard = self
      .ctx
      .cv
      .wait_while(guard, |state| *state == State::Running)
      .unwrap();
  }

  pub fn submit<F: FnOnce(&mut T) + Send + 'scope>(&self, f: F) {
    let mut guard = self.ctx.state.lock().unwrap();
    assert_eq!(*guard, State::Ready);
    *guard = State::Running;
    self.sender.send(Box::new(f)).unwrap();
    guard = self
      .ctx
      .cv
      .wait_while(guard, |state| *state == State::Running)
      .unwrap();
  }

  pub fn join(self) {
    while self.is_paused() {
      self.unpause();
    }
    drop(self.sender);
    self.inner.join().unwrap();
  }
}
