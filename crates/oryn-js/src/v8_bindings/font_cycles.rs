// Native font loading cycle identities. Pure state transitions contain no V8,
// parser, broker or author callbacks. Promise settlement is an owning-realm
// transaction performed by the binding adapter after revalidating this token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FaceId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Attempt { pub face: FaceId, pub revision: u64, pub serial: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status { Unloaded, Loading, Loaded, Error }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Terminal { Decoded, Exhausted, Cancelled }
pub(crate) struct Face {
    pub revision: u64,
    pub serial: u64,
    pub status: Status,
}
#[derive(Default)]
pub(crate) struct Cycles {
    next_face: u64,
    next_attempt: u64,
    cycle: u64,
    retired: bool,
    faces: std::collections::BTreeMap<FaceId, Face>,
    loading: usize,
}
impl Cycles {
    pub fn create(&mut self) -> Result<FaceId, &'static str> {
        if self.retired { return Err("font document retired"); }
        let next = self.next_face.checked_add(1).ok_or("font identity exhausted")?;
        let id = FaceId(next);
        self.faces.insert(id, Face {revision:0, serial:0, status:Status::Unloaded});
        self.next_face = next;
        Ok(id)
    }
    pub fn cycle(&self) -> u64 { self.cycle }
    pub fn loading(&self) -> bool { self.loading != 0 }
    pub fn status(&self, id:FaceId) -> Option<Status> { self.faces.get(&id).map(|f|f.status) }
    pub fn revision(&self,id:FaceId)->Option<u64>{self.faces.get(&id).map(|f|f.revision)}
    pub fn attempt(&self,id:FaceId)->Option<Attempt>{self.faces.get(&id).filter(|face|face.status==Status::Loading)
        .map(|face|Attempt{face:id,revision:face.revision,serial:face.serial})}
    pub fn start(&mut self, id:FaceId) -> Result<Option<Attempt>, &'static str> {
        if self.retired { return Err("font document retired"); }
        let face = self.faces.get(&id).ok_or("font face missing")?;
        // load() returns the existing loaded promise after either terminal
        // result. Retrying a failed face must not invent a new load cycle.
        if matches!(face.status, Status::Loaded | Status::Error) { return Ok(None); }
        if face.status == Status::Loading {
            return Ok(Some(Attempt {face:id,revision:face.revision,serial:face.serial}));
        }
        let serial = self.next_attempt.checked_add(1).ok_or("font attempt exhausted")?;
        let cycle = if self.loading == 0 {self.cycle.checked_add(1).ok_or("font cycle exhausted")?} else {self.cycle};
        let loading = self.loading.checked_add(1).ok_or("font loading count exhausted")?;
        let face = self.faces.get_mut(&id).ok_or("font face missing")?;
        face.status = Status::Loading;
        face.serial = serial;
        self.next_attempt = serial;
        self.loading = loading;
        self.cycle = cycle;
        Ok(Some(Attempt {face:id,revision:face.revision,serial}))
    }
    pub fn finish(&mut self, token:Attempt, terminal:Terminal) -> bool {
        if self.retired { return false; }
        let Some(face)=self.faces.get_mut(&token.face) else {return false};
        if face.status != Status::Loading || face.serial != token.serial || face.revision != token.revision {
            return false;
        }
        face.status = if terminal == Terminal::Decoded {Status::Loaded} else {Status::Error};
        self.loading -= 1;
        true
    }
    pub fn current(&self, token:Attempt)->bool {
        !self.retired && self.faces.get(&token.face).is_some_and(|face|
            face.status==Status::Loading && face.serial==token.serial && face.revision==token.revision)
    }
    pub fn rollback_start(&mut self,token:Attempt)->bool {
        if !self.current(token){return false;}
        let face=self.faces.get_mut(&token.face).expect("current font face exists");
        face.status=Status::Unloaded;
        self.loading-=1;
        true
    }
    pub fn replace(&mut self,id:FaceId) -> Result<(), &'static str> {
        let face = self.faces.get_mut(&id).ok_or("font face missing")?;
        let revision=face.revision.checked_add(1).ok_or("font face revision exhausted")?;
        if face.status == Status::Loading {self.loading-=1;}
        face.status=Status::Unloaded;face.revision=revision;
        Ok(())
    }
    pub fn retire(&mut self) {
        self.retired=true;self.loading=0;
        for face in self.faces.values_mut() {if face.status==Status::Loading {face.status=Status::Error;}}
    }
    // Only an initial-Window Document commit may create new authority. Existing
    // records retain terminal states and monotonically increasing identities.
    pub fn replacement_document(&mut self) {
        debug_assert!(self.retired);
        self.retired=false;
    }
    pub fn remove(&mut self,id:FaceId) {
        if self.faces.remove(&id).is_some_and(|face|face.status==Status::Loading){self.loading-=1;}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_replacement_keeps_old_terminal_faces_and_unique_ids(){
        let mut cycles=Cycles::default();let old=cycles.create().unwrap();
        let attempt=cycles.start(old).unwrap().unwrap();
        cycles.retire();cycles.replacement_document();
        let new=cycles.create().unwrap();assert_ne!(old,new);
        assert_eq!(cycles.status(old),Some(Status::Error));
        assert!(!cycles.finish(attempt,Terminal::Decoded));
        assert_eq!(cycles.start(old).unwrap(),None);
        assert!(cycles.start(new).unwrap().is_some());
    }
    #[test]
    fn pending_cycle_identity_and_exact_completion() {
        let mut c=Cycles::default();let a=c.create().unwrap();let b=c.create().unwrap();
        let first=c.start(a).unwrap().unwrap();let cycle=c.cycle();
        let second=c.start(b).unwrap().unwrap();
        assert_eq!(cycle,c.cycle());assert_eq!(c.start(a).unwrap(),Some(first));
        assert!(c.finish(first,Terminal::Decoded));assert!(c.loading());
        assert!(c.finish(second,Terminal::Exhausted));assert!(!c.loading());
        assert!(!c.finish(second,Terminal::Decoded));
        assert_eq!(c.start(b).unwrap(),None);
        c.replace(b).unwrap();
        let next=c.start(b).unwrap().unwrap();assert!(c.cycle()>cycle);
        assert!(!c.finish(second,Terminal::Decoded));assert!(c.finish(next,Terminal::Decoded));
    }
    #[test]
    fn replacement_and_retirement_reject_stale_success() {
        let mut c=Cycles::default();let a=c.create().unwrap();let old=c.start(a).unwrap().unwrap();
        c.replace(a).unwrap();let new=c.start(a).unwrap().unwrap();
        assert!(!c.finish(old,Terminal::Decoded));assert!(c.loading());
        c.retire();assert!(!c.finish(new,Terminal::Decoded));assert!(!c.loading());
        assert_eq!(c.status(a),Some(Status::Error));assert!(c.start(a).is_err());
    }
}
