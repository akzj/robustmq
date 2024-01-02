// Copyright 2023 RobustMQ Team
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use common::config::meta::MetaConfig;
use rocksdb::SliceTransform;
use rocksdb::{ColumnFamily, DBCompactionStyle, Options, DB};
use serde::{de::DeserializeOwned, Serialize};
use serde_json;
use std::collections::HashMap;
use std::path::Path;

use super::{DB_COLUMN_FAMILY_CLUSTER, column_family_list};
use super::DB_COLUMN_FAMILY_META;
use super::DB_COLUMN_FAMILY_MQTT;

pub struct RocksDBStorage {
    db: DB,
}

impl RocksDBStorage {
    /// Create a rocksdb instance
    pub fn new(config: &MetaConfig) -> Self {
        let opts: Options = Self::open_db_opts(config);
        let db_path = format!("{}/{}", config.data_path, "_storage_rocksdb");

        // init RocksDB
        if !Path::new(&db_path).exists() {
            DB::open(&opts, db_path.clone()).unwrap();
        }

        // init column family
        let cf_list = rocksdb::DB::list_cf(&opts, &db_path).unwrap();
        let mut instance = DB::open_cf(&opts, db_path.clone(), &cf_list).unwrap();

        for family in column_family_list().iter() {
            if cf_list.iter().find(|cf| cf == &family).is_none() {
                instance.create_cf(&family, &opts).unwrap();
            }
        }

        return RocksDBStorage { db: instance };
    }

    /// Write the data serialization to RocksDB
    pub fn write<T: Serialize + std::fmt::Debug>(
        &self,
        cf: &ColumnFamily,
        key: &str,
        value: &T,
    ) -> Result<(), String> {
        match serde_json::to_string(&value) {
            Ok(serialized) => self
                .db
                .put_cf(cf, key, serialized.into_bytes())
                .map_err(|err| format!("Failed to put to ColumnFamily:{:?}", err)),
            Err(err) => Err(format!(
                "Failed to serialize to String. T: {:?}, err: {:?}",
                value, err
            )),
        }
    }

    // Read data from the RocksDB
    pub fn read<T: DeserializeOwned>(
        &self,
        cf: &ColumnFamily,
        key: &str,
    ) -> Result<Option<T>, String> {
        match self.db.get_cf(cf, key) {
            Ok(opt) => match opt {
                Some(found) => match String::from_utf8(found) {
                    Ok(s) => match serde_json::from_str::<T>(&s) {
                        Ok(t) => Ok(Some(t)),
                        Err(err) => Err(format!("Failed to deserialize: {:?}", err)),
                    },
                    Err(err) => Err(format!("Failed to deserialize: {:?}", err)),
                },
                None => Ok(None),
            },
            Err(err) => Err(format!("Failed to get from ColumnFamily: {:?}", err)),
        }
    }

    // Search data by prefix
    pub fn read_prefix(&self, cf: &ColumnFamily, key: &str) -> Vec<Vec<Vec<u8>>> {
        let mut iter = self.db.raw_iterator_cf(cf);
        iter.seek(key);
        println!("key={}", key);
        let mut result: Vec<Vec<Vec<u8>>> = Vec::new();
        while iter.valid() {
            let key = iter.key();
            let value = iter.value();

            let mut raw: Vec<Vec<u8>> = Vec::new();
            if key == None || value == None {
                continue;
            }
            raw.push(key.unwrap().to_vec());
            raw.push(value.unwrap().to_vec());

            result.push(raw);

            iter.next();
        }
        return result;
    }

    // Read data from all Columnfamiliy
    pub fn read_all(&self) -> HashMap<String, Vec<Vec<Vec<u8>>>> {
        let mut result: HashMap<String, Vec<Vec<Vec<u8>>>> = HashMap::new();
        for family in column_family_list().iter() {
            let cf = if *family == DB_COLUMN_FAMILY_META {
                self.cf_meta()
            } else if *family == DB_COLUMN_FAMILY_CLUSTER {
                self.cf_cluster()
            } else {
                self.cf_mqtt()
            };
            result.insert(family.to_string(), self.read_all_by_cf(cf));
        }
        return result;
    }

    // Read all data in a ColumnFamily
    pub fn read_all_by_cf(&self, cf: &ColumnFamily) -> Vec<Vec<Vec<u8>>> {
        let mut iter = self.db.raw_iterator_cf(cf);
        iter.seek_to_first();

        let mut result: Vec<Vec<Vec<u8>>> = Vec::new();
        while iter.valid() {
            let key = iter.key();
            let value = iter.value();

            let mut raw: Vec<Vec<u8>> = Vec::new();
            if key == None || value == None {
                continue;
            }
            raw.push(key.unwrap().to_vec());
            raw.push(value.unwrap().to_vec());

            result.push(raw);

            iter.next();
        }
        return result;
    }

    pub fn delete(&self, cf: &ColumnFamily, key: &str) -> Result<(), String> {
        match self.db.delete_cf(cf, key) {
            Ok(_) => Ok(()),
            Err(err) => Err(format!("Failed to delete from ColumnFamily: {:?}", err)),
        }
    }

    pub fn cf_meta(&self) -> &ColumnFamily {
        return self.db.cf_handle(&DB_COLUMN_FAMILY_META).unwrap();
    }

    pub fn cf_cluster(&self) -> &ColumnFamily {
        return self.db.cf_handle(&DB_COLUMN_FAMILY_CLUSTER).unwrap();
    }

    pub fn cf_mqtt(&self) -> &ColumnFamily {
        return self.db.cf_handle(&DB_COLUMN_FAMILY_MQTT).unwrap();
    }

    fn open_db_opts(config: &MetaConfig) -> Options {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_max_open_files(config.rocksdb.max_open_files.unwrap());
        opts.set_use_fsync(false);
        opts.set_bytes_per_sync(8388608);
        opts.optimize_for_point_lookup(1024);
        opts.set_table_cache_num_shard_bits(6);
        opts.set_max_write_buffer_number(32);
        opts.set_write_buffer_size(536870912);
        opts.set_target_file_size_base(1073741824);
        opts.set_min_write_buffer_number_to_merge(4);
        opts.set_level_zero_stop_writes_trigger(2000);
        opts.set_level_zero_slowdown_writes_trigger(0);
        opts.set_compaction_style(DBCompactionStyle::Universal);
        opts.set_disable_auto_compactions(true);

        let transform = SliceTransform::create_fixed_prefix(10);
        opts.set_prefix_extractor(transform);
        opts.set_memtable_prefix_bloom_ratio(0.2);

        return opts;
    }


    
}

#[cfg(test)]
mod tests {

    use super::RocksDBStorage;
    use common::config::meta::MetaConfig;
    use serde::{Deserialize, Serialize};
    use tokio::fs::remove_dir;

    #[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
    struct User {
        pub name: String,
        pub age: u32,
    }

    #[tokio::test]
    async fn init_family() {
        let config = MetaConfig::default();
        let rs = RocksDBStorage::new(&config);
        let key = "name2";
        let res1 = rs.read::<User>(rs.cf_meta(), key);
        assert!(res1.unwrap().is_none());

        let user = User {
            name: "lobo".to_string(),
            age: 18,
        };
        let res4 = rs.write(rs.cf_meta(), key, &user);
        assert!(res4.is_ok());

        let res5 = rs.read::<User>(rs.cf_meta(), key);
        let res5_rs = res5.unwrap().unwrap();
        assert!(res5_rs.name == user.name);
        assert!(res5_rs.age == user.age);

        let res6 = rs.delete(rs.cf_meta(), key);
        assert!(res6.is_ok());

        match remove_dir(config.data_path).await {
            Ok(_) => {}
            Err(err) => {
                println!("{:?}", err)
            }
        }
    }

    #[tokio::test]
    async fn read_all() {
        let config = MetaConfig::default();
        let rs = RocksDBStorage::new(&config);
        let result = rs.read_all_by_cf(rs.cf_meta());
        for raw in result.clone() {
            let key = raw.get(0).unwrap().to_vec();
            let value = raw.get(1).unwrap().to_vec();
            println!(
                "key={}, value={}",
                String::from_utf8(key).unwrap(),
                String::from_utf8(value).unwrap()
            );
        }
        println!("size:{}", result.len());
    }

    #[tokio::test]
    async fn read_prefix() {
        let config = MetaConfig::default();
        let rs = RocksDBStorage::new(&config);
        let result = rs.read_prefix(rs.cf_meta(), "metasrv_conf");
        for raw in result.clone() {
            let key = raw.get(0).unwrap().to_vec();
            let value = raw.get(1).unwrap().to_vec();
            println!(
                "key={}, value={}",
                String::from_utf8(key).unwrap(),
                String::from_utf8(value).unwrap()
            );
        }
        println!("size:{}", result.len());
    }
}
