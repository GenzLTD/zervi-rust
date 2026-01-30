pub mod user;

pub use user::*;

// 重新导出常用类型
pub type UserModel = user::Model;

// 其他服务的模型在auth-service中不需要
// 如果需要，通过API调用其他服务
