use std::f64;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::Read;
use std::mem::zeroed;
use std::os::raw::{c_char, c_int};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::slice;

use anyhow::{bail, Result};
use crfsuite_sys::crfsuite_create_instance_from_memory;
use crfsuite_sys::floatval_t;

#[derive(Debug)]
pub struct SimpleAttribute {
    pub attr: String,
    pub value: f64,
}

pub trait Attribute {
    fn get_attr(&self) -> Result<CString>;
    fn get_value(&self) -> f64;
}

impl Attribute for SimpleAttribute {
    fn get_attr(&self) -> Result<CString> {
        Ok(CString::new(self.attr.as_bytes())?)
    }

    fn get_value(&self) -> f64 {
        self.value
    }
}

impl Attribute for (String, String) {
    fn get_attr(&self) -> Result<CString> {
        let &(ref key, ref value) = self;
        Ok(CString::new(format!("{}:{}", key, value).as_bytes())?)
    }

    fn get_value(&self) -> f64 {
        1.0
    }
}

pub struct Tagger {
    #[allow(unused)]
    // we own the bytes here that is used in the C code
    bytes: Vec<u8>,
    model: ModelWrapper,
    tagger: TaggerWrapper,
}

impl Tagger {
    pub fn create_from_file<P: AsRef<Path>>(path: P) -> Result<Tagger> {
        let mut file = File::open(path)?;
        let mut bytes = Vec::with_capacity(file.metadata()?.len() as usize);
        file.read_to_end(&mut bytes)?;

        Tagger::create_from_memory(bytes)
    }

    pub fn create_from_memory(data: Vec<u8>) -> Result<Tagger> {
        let mut model = null_mut();

        let r = unsafe {
            let x: &[u8] = data.as_ref();
            crfsuite_create_instance_from_memory(
                x.as_ptr() as *const _,
                data.len() as libc::size_t,
                &mut model,
            )
        };

        if r != 0 {
            bail!("error while creating instance : non zero C return code...")
        }

        let model: *mut crfsuite_sys::crfsuite_model_t = model as *mut _;

        let model = ModelWrapper { model };

        let mut tagger = null_mut();

        let r = model.get_tagger(&mut tagger);
        if r != 0 {
            bail!("error while getting tagger : non zero C return code...")
        }

        Ok(Tagger {
            model,
            tagger: TaggerWrapper { tagger },
            bytes: data,
        })
    }

    pub fn labels(&self) -> Result<Vec<String>> {
        let mut labels = null_mut();

        let r = self.model.get_labels(&mut labels);
        if r != 0 {
            // TODO try to call release raw labels pointer ?
            bail!("failed to obtain the dictionary interface for labels")
        }

        let labels = DictionaryWrapper { dict: labels };

        let mut lseq = Vec::with_capacity(labels.num() as usize);

        for i in 0..labels.num() {
            let mut label = null();
            let r = labels.id_to_string(i, &mut label);
            if r != 0 {
                bail!("failed to convert a label identifier to string")
            }

            lseq.push(unsafe { CStr::from_ptr(label) }.to_str()?.to_string());

            labels.free(label);
        }

        Ok(lseq)
    }

    pub fn tag<A: Attribute>(&self, input: &[Vec<A>]) -> Result<Vec<String>> {
        self.set(input)?;
        self.viterbi()
    }

    pub fn set<A: Attribute>(&self, input: &[Vec<A>]) -> Result<()> {
        let mut attrs = null_mut();
        let r = self.model.get_attrs(&mut attrs);
        if r != 0 {
            bail!("error while getting tagger : non zero C return code...")
        }
        let attrs = DictionaryWrapper { dict: attrs };
        let mut inst = unsafe { zeroed() };

        unsafe {
            crfsuite_sys::crfsuite_instance_init_n(&mut inst, input.len() as libc::c_int);
        }

        let inst_items = unsafe { slice::from_raw_parts_mut(inst.items, inst.num_items as usize) };

        for (i, item) in input.iter().enumerate() {
            let inst_item = &mut inst_items[i];

            unsafe { crfsuite_sys::crfsuite_item_init(inst_item) };

            for inner_item in item.iter() {
                let raw_pointer = inner_item.get_attr()?.into_raw();
                let aid = attrs.str_to_id(raw_pointer);

                if 0 <= aid {
                    let cont = &mut unsafe { zeroed() };
                    unsafe {
                        crfsuite_sys::crfsuite_attribute_set(cont, aid, inner_item.get_value())
                    };
                    unsafe { crfsuite_sys::crfsuite_item_append_attribute(inst_item, cont) };
                }

                let _ = unsafe { CString::from_raw(raw_pointer) }; // get back the string to free it
            }
        }

        let r = self.tagger.set(&mut inst);

        if r != 0 {
            unsafe {
                crfsuite_sys::crfsuite_instance_finish(&mut inst);
            }
            bail!("error while getting tagger : non zero C return code...")
        }

        unsafe {
            crfsuite_sys::crfsuite_instance_finish(&mut inst);
        }

        Ok(())
    }

    pub fn viterbi(&self) -> Result<Vec<String>> {
        let t: usize = self.tagger.length() as usize;
        if t == 0 {
            return Ok(vec![]);
        }

        let mut labels = null_mut();

        let r = self.model.get_labels(&mut labels);
        if r != 0 {
            // TODO try to call release raw labels pointer ?
            bail!("failed to obtain the dictionary interface for labels")
        }

        let labels = DictionaryWrapper { dict: labels };

        let mut score = f64::NAN;
        let mut path = vec![0; t];

        let r = self.tagger.viterbi(&mut path[0], &mut score);
        if r != 0 {
            bail!("failed to find the viterbi path")
        }

        let mut yseq = Vec::with_capacity(t);

        for p in path.into_iter().take(t) {
            let mut label = null();
            let r = labels.id_to_string(p, &mut label);
            if r != 0 {
                bail!("failed to convert a label identifier to string")
            }

            yseq.push(unsafe { CStr::from_ptr(label) }.to_str()?.to_string());

            labels.free(label);
        }
        Ok(yseq)
    }

    pub fn probability(&self, tags: &[String]) -> Result<f64> {
        let t: usize = self.tagger.length() as usize;
        if t == 0 {
            return Ok(0.0);
        }
        if t != tags.len() {
            bail!(
                "The number of items and labels differ |x| = {}, |y| = {}",
                t,
                tags.len()
            );
        }

        let mut labels = null_mut();

        let r = self.model.get_labels(&mut labels);
        if r != 0 {
            // TODO try to call release raw labels pointer ?
            bail!("Failed to obtain the dictionary interface for labels")
        }

        let labels = DictionaryWrapper { dict: labels };

        let mut path = vec![0; t];

        for i in 0..t {
            let l = labels.str_to_id(CString::new(tags[i].as_bytes())?.into_raw());
            if l < 0 {
                bail!("Failed to convert into label identifier : {}", tags[i]);
            }
            path[i] = l;
        }

        let mut score = f64::NAN;

        let r = self.tagger.score(&mut path[0], &mut score);
        if r != 0 {
            bail!("Failed to score the label sequence")
        }

        let mut lognorm = f64::NAN;

        let r = self.tagger.lognorm(&mut lognorm);
        if r != 0 {
            bail!("Failed to compute the partition factor")
        }

        Ok((score - lognorm).exp())
    }

    /*pub fn marginal(&self, label: &str, position: usize) -> f64 {
        unimplemented!();
    }*/
}

struct DictionaryWrapper {
    dict: *mut crfsuite_sys::crfsuite_dictionary_t,
}

// see https://github.com/chokkan/crfsuite/issues/35 send should not pose any problems
unsafe impl Send for DictionaryWrapper {}

impl DictionaryWrapper {
    fn str_to_id(&self, str: *const c_char) -> c_int {
        unsafe {
            if let Some(to_id) = (*self.dict).to_id {
                to_id(self.dict, str)
            } else {
                panic!("no callback for to_id")
            }
        }
    }

    fn id_to_string(&self, id: c_int, pstr: *mut *const c_char) -> c_int {
        unsafe {
            if let Some(to_string) = (*self.dict).to_string {
                to_string(self.dict, id, pstr)
            } else {
                panic!("no callback for to_string")
            }
        }
    }

    fn free(&self, str: *const c_char) {
        unsafe {
            if let Some(free) = (*self.dict).free {
                free(self.dict, str)
            } else {
                panic!("no callback for free")
            }
        }
    }

    fn num(&self) -> c_int {
        unsafe {
            if let Some(num) = (*self.dict).num {
                num(self.dict)
            } else {
                panic!("no callback for num")
            }
        }
    }
}

impl Drop for DictionaryWrapper {
    fn drop(&mut self) {
        unsafe {
            if let Some(release) = (*self.dict).release {
                release(self.dict);
            } else {
                panic!("no callback for release")
            }
        }
    }
}

struct TaggerWrapper {
    // TODO : ensure thread safety
    tagger: *mut crfsuite_sys::crfsuite_tagger_t,
}

// see https://github.com/chokkan/crfsuite/issues/35 send should not pose any problems
unsafe impl Send for TaggerWrapper {}

impl TaggerWrapper {
    fn set(&self, inst: *mut crfsuite_sys::crfsuite_instance_t) -> c_int {
        unsafe {
            if let Some(set) = (*self.tagger).set {
                set(self.tagger, inst)
            } else {
                panic!("no callback for set")
            }
        }
    }

    fn length(&self) -> ::std::os::raw::c_int {
        unsafe {
            if let Some(length) = (*self.tagger).length {
                length(self.tagger)
            } else {
                panic!("no callback for length")
            }
        }
    }

    fn viterbi(&self, labels: *mut c_int, ptr_score: *mut floatval_t) -> c_int {
        unsafe {
            if let Some(viterbi) = (*self.tagger).viterbi {
                viterbi(self.tagger, labels, ptr_score)
            } else {
                panic!("no callback for viterbi")
            }
        }
    }

    fn score(&self, path: *mut c_int, ptr_score: *mut floatval_t) -> c_int {
        unsafe {
            if let Some(score) = (*self.tagger).score {
                score(self.tagger, path, ptr_score)
            } else {
                panic!("no callback for score")
            }
        }
    }

    fn lognorm(&self, ptr_norm: *mut floatval_t) -> c_int {
        unsafe {
            if let Some(lognorm) = (*self.tagger).lognorm {
                lognorm(self.tagger, ptr_norm)
            } else {
                panic!("no callback for lognorm")
            }
        }
    }
}

impl Drop for TaggerWrapper {
    fn drop(&mut self) {
        unsafe {
            if let Some(release) = (*self.tagger).release {
                release(self.tagger);
            } else {
                panic!("no callback for release")
            }
        }
    }
}

struct ModelWrapper {
    model: *mut crfsuite_sys::crfsuite_model_t,
}

// see https://github.com/chokkan/crfsuite/issues/35 send should not pose any problems
unsafe impl Send for ModelWrapper {}

impl ModelWrapper {
    pub fn get_tagger(&self, ptr_tagger: *mut *mut crfsuite_sys::crfsuite_tagger_t) -> c_int {
        unsafe {
            if let Some(get_tagger) = (*self.model).get_tagger {
                get_tagger(self.model, ptr_tagger)
            } else {
                panic!("no callback for get_tagger")
            }
        }
    }

    pub fn get_labels(&self, ptr_labels: *mut *mut crfsuite_sys::crfsuite_dictionary_t) -> c_int {
        unsafe {
            if let Some(get_labels) = (*self.model).get_labels {
                get_labels(self.model, ptr_labels)
            } else {
                panic!("no callback for get_labels")
            }
        }
    }

    pub fn get_attrs(&self, ptr_attrs: *mut *mut crfsuite_sys::crfsuite_dictionary_t) -> c_int {
        unsafe {
            if let Some(get_attrs) = (*self.model).get_attrs {
                get_attrs(self.model, ptr_attrs)
            } else {
                panic!("no callback for get_labels")
            }
        }
    }
}

impl Drop for ModelWrapper {
    fn drop(&mut self) {
        unsafe {
            if let Some(release) = (*self.model).release {
                release(self.model);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SimpleAttribute;
    use super::Tagger;
    use std::env;
    use std::fs::File;
    use std::io::Read;
    use std::path;

    #[test]
    fn tagger_works() {
        let t = Tagger::create_from_file(file_path("modela78m0U.crfsuite"));

        let input = vec![
            vec![
                SimpleAttribute {
                    attr: "is_first:1".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_1:Xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_2:Xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1:set".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters:01010000000".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+1]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[+1]:11110111111111".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+2]:to".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_2[+1]:rare_word to".to_string(),
                    value: 1.0,
                },
            ],
            vec![
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-1]:01010000000".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_2:xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_2[-1]:Xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_1:xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_3[-1]:Xxx xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-1]:set".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters:11110111111111".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+1]:to".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_first[-1]:1".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[+1]:1010".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+2]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_2[+1]:to rare_word".to_string(),
                    value: 1.0,
                },
            ],
            vec![
                SimpleAttribute {
                    attr: "is_first[-2]:1".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-1]:11110111111111".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-2]:set".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-2]:01010000000".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_2[-1]:xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_last[+2]:1".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_2[-2]:set rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_1:xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_2:xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_3[-1]:xxx xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1:to".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-1]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters:1010".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+1]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[+1]:11111110100".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+2]:please".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_digit[+1]:1".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_2[+1]:rare_word please".to_string(),
                    value: 1.0,
                },
            ],
            vec![
                SimpleAttribute {
                    attr: "ngram_2[-2]:rare_word to".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-1]:1010".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[+1]:11101010110".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-2]:11110111111111".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_2[-1]:xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_digit:1".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-1]:to".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_last[+1]:1".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters:11111110100".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+1]:please".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-2]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_1:xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "built-in-snips/number:U-".to_string(),
                    value: 1.0,
                },
            ],
            vec![
                SimpleAttribute {
                    attr: "ngram_2[-2]:to rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-1]:11111110100".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "built-in-snips/number[-1]:U-".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-2]:1010".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_digit[-1]:1".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-1]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_last:1".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters:11101010110".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1:please".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-2]:to".to_string(),
                    value: 1.0,
                },
            ],
        ];

        let r = t.unwrap().tag(&input);

        assert_eq!(r.unwrap(), vec!["O", "O", "O", "B-target-en", "O"]);
    }

    #[test]
    fn tagger_kv_works() {
        let t = Tagger::create_from_file(file_path("modela78m0U.crfsuite"));

        let input = vec![
            vec![
                ("is_first".to_string(), "1".to_string()),
                ("shape_ngram_1".to_string(), "Xxx".to_string()),
                ("shape_ngram_2".to_string(), "Xxx xxx".to_string()),
                ("ngram_1".to_string(), "set".to_string()),
                (
                    "word_cluster_brown_clusters".to_string(),
                    "01010000000".to_string(),
                ),
                ("ngram_1[+1]".to_string(), "rare_word".to_string()),
                (
                    "word_cluster_brown_clusters[+1]".to_string(),
                    "11110111111111".to_string(),
                ),
                ("ngram_1[+2]".to_string(), "to".to_string()),
                ("ngram_2[+1]".to_string(), "rare_word to".to_string()),
            ],
            vec![
                (
                    "word_cluster_brown_clusters[-1]".to_string(),
                    "01010000000".to_string(),
                ),
                ("shape_ngram_2".to_string(), "xxx xxx".to_string()),
                ("shape_ngram_2[-1]".to_string(), "Xxx xxx".to_string()),
                ("shape_ngram_1".to_string(), "xxx".to_string()),
                ("shape_ngram_3[-1]".to_string(), "Xxx xxx xxx".to_string()),
                ("ngram_1".to_string(), "rare_word".to_string()),
                ("ngram_1[-1]".to_string(), "set".to_string()),
                (
                    "word_cluster_brown_clusters".to_string(),
                    "11110111111111".to_string(),
                ),
                ("ngram_1[+1]".to_string(), "to".to_string()),
                ("is_first[-1]".to_string(), "1".to_string()),
                (
                    "word_cluster_brown_clusters[+1]".to_string(),
                    "1010".to_string(),
                ),
                ("ngram_1[+2]".to_string(), "rare_word".to_string()),
                ("ngram_2[+1]".to_string(), "to rare_word".to_string()),
            ],
            vec![
                ("is_first[-2]".to_string(), "1".to_string()),
                (
                    "word_cluster_brown_clusters[-1]".to_string(),
                    "11110111111111".to_string(),
                ),
                ("ngram_1[-2]".to_string(), "set".to_string()),
                (
                    "word_cluster_brown_clusters[-2]".to_string(),
                    "01010000000".to_string(),
                ),
                ("shape_ngram_2[-1]".to_string(), "xxx xxx".to_string()),
                ("is_last[+2]".to_string(), "1".to_string()),
                ("ngram_2[-2]".to_string(), "set rare_word".to_string()),
                ("shape_ngram_1".to_string(), "xxx".to_string()),
                ("shape_ngram_2".to_string(), "xxx xxx".to_string()),
                ("shape_ngram_3[-1]".to_string(), "xxx xxx xxx".to_string()),
                ("ngram_1".to_string(), "to".to_string()),
                ("ngram_1[-1]".to_string(), "rare_word".to_string()),
                (
                    "word_cluster_brown_clusters".to_string(),
                    "1010".to_string(),
                ),
                ("ngram_1[+1]".to_string(), "rare_word".to_string()),
                (
                    "word_cluster_brown_clusters[+1]".to_string(),
                    "11111110100".to_string(),
                ),
                ("ngram_1[+2]".to_string(), "please".to_string()),
                ("is_digit[+1]".to_string(), "1".to_string()),
                ("ngram_2[+1]".to_string(), "rare_word please".to_string()),
            ],
            vec![
                ("ngram_2[-2]".to_string(), "rare_word to".to_string()),
                (
                    "word_cluster_brown_clusters[-1]".to_string(),
                    "1010".to_string(),
                ),
                (
                    "word_cluster_brown_clusters[+1]".to_string(),
                    "11101010110".to_string(),
                ),
                (
                    "word_cluster_brown_clusters[-2]".to_string(),
                    "11110111111111".to_string(),
                ),
                ("shape_ngram_2[-1]".to_string(), "xxx xxx".to_string()),
                ("is_digit".to_string(), "1".to_string()),
                ("ngram_1".to_string(), "rare_word".to_string()),
                ("ngram_1[-1]".to_string(), "to".to_string()),
                ("is_last[+1]".to_string(), "1".to_string()),
                (
                    "word_cluster_brown_clusters".to_string(),
                    "11111110100".to_string(),
                ),
                ("ngram_1[+1]".to_string(), "please".to_string()),
                ("ngram_1[-2]".to_string(), "rare_word".to_string()),
                ("shape_ngram_1".to_string(), "xxx".to_string()),
                ("built-in-snips/number".to_string(), "U-".to_string()),
            ],
            vec![
                ("ngram_2[-2]".to_string(), "to rare_word".to_string()),
                (
                    "word_cluster_brown_clusters[-1]".to_string(),
                    "11111110100".to_string(),
                ),
                ("built-in-snips/number[-1]".to_string(), "U-".to_string()),
                (
                    "word_cluster_brown_clusters[-2]".to_string(),
                    "1010".to_string(),
                ),
                ("is_digit[-1]".to_string(), "1".to_string()),
                ("ngram_1[-1]".to_string(), "rare_word".to_string()),
                ("is_last".to_string(), "1".to_string()),
                (
                    "word_cluster_brown_clusters".to_string(),
                    "11101010110".to_string(),
                ),
                ("ngram_1".to_string(), "please".to_string()),
                ("ngram_1[-2]".to_string(), "to".to_string()),
            ],
        ];

        let r = t.unwrap().tag(&input);

        assert_eq!(r.unwrap(), vec!["O", "O", "O", "B-target-en", "O"]);
    }

    #[test]
    fn probability_works() {
        let t = Tagger::create_from_file(file_path("modelo62R_B.crfsuite")).unwrap();

        let input = vec![
            vec![
                SimpleAttribute {
                    attr: "is_first:1".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_1:Xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_2:Xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_in_gazetteer_states_us[+1]:U-".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters:11110111110000".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+1]:me".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[+1]:01010011100".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+2]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_2[+1]:me rare_word".to_string(),
                    value: 1.0,
                },
            ],
            vec![
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-1]:11110111110000".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_2:xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_2[-1]:Xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_in_gazetteer_states_us:U-".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_1:xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_3[-1]:Xxx xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1:me".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-1]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters:01010011100".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+1]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_first[-1]:1".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[+1]:11111110101111".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+2]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_2[+1]:rare_word rare_word".to_string(),
                    value: 1.0,
                },
            ],
            vec![
                SimpleAttribute {
                    attr: "is_first[-2]:1".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-1]:01010011100".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[+1]:11110011011100".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-2]:11110111110000".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_2[-1]:xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_2[-2]:rare_word me".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "built-in-snips/number:U-".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_2:xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_3[-1]:xxx xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_in_gazetteer_states_us[-1]:U-".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-1]:me".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_1:xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters:11111110101111".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+1]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-2]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+2]:of".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_2[+1]:rare_word of".to_string(),
                    value: 1.0,
                },
            ],
            vec![
                SimpleAttribute {
                    attr: "ngram_2[-2]:me rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-1]:11111110101111".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "built-in-snips/number[-1]:U-".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-2]:01010011100".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_2[-1]:xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_last[+2]:1".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_1:xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[+1]:10110".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_3[-1]:xxx xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_in_gazetteer_cities_world[+1]:U-".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-1]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_2:xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters:11110011011100".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+1]:of".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-2]:me".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+2]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_2[+1]:of rare_word".to_string(),
                    value: 1.0,
                },
            ],
            vec![
                SimpleAttribute {
                    attr: "ngram_2[-2]:rare_word rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-1]:11110011011100".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[+1]:1110010101".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-2]:11111110101111".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_2[-1]:xxx xxx".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "built-in-snips/number[-2]:U-".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_in_gazetteer_cities_world:U-".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1:of".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-1]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_last[+1]:1".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters:10110".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[+1]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-2]:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "shape_ngram_1:xxx".to_string(),
                    value: 1.0,
                },
            ],
            vec![
                SimpleAttribute {
                    attr: "ngram_2[-2]:rare_word of".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-1]:10110".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters[-2]:11110011011100".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_in_gazetteer_cities_world[-1]:U-".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-1]:of".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "is_last:1".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "word_cluster_brown_clusters:1110010101".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1:rare_word".to_string(),
                    value: 1.0,
                },
                SimpleAttribute {
                    attr: "ngram_1[-2]:rare_word".to_string(),
                    value: 1.0,
                },
            ],
        ];

        t.set(&input).unwrap();

        let p1 = t
            .probability(&[
                "O".to_string(),
                "O".to_string(),
                "B-snips/number".to_string(),
                "O".to_string(),
                "O".to_string(),
                "O".to_string(),
            ])
            .unwrap();

        assert!(p1.is_finite());
        assert!(p1 - 0.999_977_801_144 < 1e-6);

        let p2 = t
            .probability(&[
                "O".to_string(),
                "O".to_string(),
                "O".to_string(),
                "O".to_string(),
                "O".to_string(),
                "O".to_string(),
            ])
            .unwrap();

        assert!(p2.is_finite());
        assert!(p2 - 9.730_620_958_25e-06 < 1e-12)
    }

    #[test]
    fn labels_work() {
        let t = Tagger::create_from_file(file_path("modelo62R_B.crfsuite")).unwrap();
        let labels = t.labels().unwrap();
        assert_eq!(labels, vec!["O", "B-snips/number", "I-snips/number"]);
    }

    #[test]
    fn create_from_memory_work() {
        fn create_tagger() -> Tagger {
            // create the tagger in a separate scope than the one we'll use it in
            let mut file = File::open(file_path("modelo62R_B.crfsuite")).unwrap();
            let mut bytes = Vec::with_capacity(file.metadata().unwrap().len() as usize);
            file.read_to_end(&mut bytes).unwrap();
            Tagger::create_from_memory(bytes).unwrap()
        }

        let t = create_tagger();

        let labels = t.labels().unwrap();
        assert_eq!(labels, vec!["O", "B-snips/number", "I-snips/number"]);

        let input = vec![vec![("is_first".to_string(), "1".to_string())]];

        let r = t.tag(&input).unwrap();

        assert_eq!(r, vec!["O"]);
    }

    pub fn file_path(file_name: &str) -> path::PathBuf {
        if env::var("DINGHY").is_ok() {
            env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .join("test_data/data")
                .join(file_name)
        } else {
            path::PathBuf::from("test-data").join(file_name)
        }
    }
}
