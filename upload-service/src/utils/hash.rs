use sha2::{Digest, Sha256};

/// 计算文件Hash (SHA256)
pub fn calculate_file_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_file_hash() {
        let data = b"test data";
        let hash1 = calculate_file_hash(data);
        let hash2 = calculate_file_hash(data);

        // 相同数据应该产生相同的hash
        assert_eq!(hash1, hash2);

        // Hash长度应该是64个字符（SHA256）
        assert_eq!(hash1.len(), 64);
    }

    #[test]
    fn test_different_data_different_hash() {
        let data1 = b"data1";
        let data2 = b"data2";

        let hash1 = calculate_file_hash(data1);
        let hash2 = calculate_file_hash(data2);

        // 不同数据应该产生不同的hash
        assert_ne!(hash1, hash2);
    }
}
