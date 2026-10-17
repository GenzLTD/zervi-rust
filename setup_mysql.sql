-- 终端自洽测试所需的最小 schema
-- adirp_user: user-service 按 sub 建档的库
CREATE DATABASE IF NOT EXISTS adirp_user DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
CREATE DATABASE IF NOT EXISTS adirp_upload DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

USE adirp_user;

CREATE TABLE IF NOT EXISTS users (
  id            VARCHAR(36)  NOT NULL,
  phone         VARCHAR(255) NOT NULL,
  email         VARCHAR(255) DEFAULT NULL,
  password_hash VARCHAR(255) NOT NULL,   -- 按 sub 建档时置空串，代表「未设置」
  name          VARCHAR(255) NOT NULL,
  avatar        VARCHAR(255) DEFAULT NULL,
  role          VARCHAR(20)  NOT NULL,   -- candidate | recruiter | admin
  status        VARCHAR(20)  NOT NULL,   -- active | disabled
  created_at    DATETIME     NOT NULL,
  updated_at    DATETIME     NOT NULL,
  PRIMARY KEY (id),
  UNIQUE KEY uq_phone (phone)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
