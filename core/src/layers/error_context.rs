// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use std::fmt::Debug;
use std::fmt::Formatter;

use futures::TryFutureExt;

use crate::raw::oio::ListOperation;
use crate::raw::oio::ReadOperation;
use crate::raw::oio::WriteOperation;
use crate::raw::*;
use crate::*;

/// ErrorContextLayer will add error context into all layers.
///
/// # Notes
///
/// This layer will adding the following error context into all errors:
///
/// - `service`: The [`Scheme`] of underlying service.
/// - `operation`: The [`Operation`] of this operation
/// - `path`: The path of this operation
///
/// Some operations may have additional context:
///
/// - `range`: The range the read operation is trying to read.
/// - `read`: The already read size in given reader.
/// - `size`: The size of the current write operation.
/// - `written`: The already written size in given writer.
/// - `listed`: The already listed size in given lister.
pub struct ErrorContextLayer;

impl<A: Access> Layer<A> for ErrorContextLayer {
    type LayeredAccess = ErrorContextAccessor<A>;

    fn layer(&self, inner: A) -> Self::LayeredAccess {
        let meta = inner.info();
        ErrorContextAccessor { meta, inner }
    }
}

/// Provide error context wrapper for backend.
pub struct ErrorContextAccessor<A: Access> {
    meta: AccessorInfo,
    inner: A,
}

impl<A: Access> Debug for ErrorContextAccessor<A> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

impl<A: Access> LayeredAccess for ErrorContextAccessor<A> {
    type Inner = A;
    type Reader = ErrorContextWrapper<A::Reader>;
    type BlockingReader = ErrorContextWrapper<A::BlockingReader>;
    type Writer = ErrorContextWrapper<A::Writer>;
    type BlockingWriter = ErrorContextWrapper<A::BlockingWriter>;
    type Lister = ErrorContextWrapper<A::Lister>;
    type BlockingLister = ErrorContextWrapper<A::BlockingLister>;

    fn inner(&self) -> &Self::Inner {
        &self.inner
    }

    fn metadata(&self) -> AccessorInfo {
        self.meta.clone()
    }

    async fn create_dir(&self, path: &str, args: OpCreateDir) -> Result<RpCreateDir> {
        self.inner
            .create_dir(path, args)
            .map_err(|err| {
                err.with_operation(Operation::CreateDir)
                    .with_context("service", self.meta.scheme())
                    .with_context("path", path)
            })
            .await
    }

    async fn read(&self, path: &str, args: OpRead) -> Result<(RpRead, Self::Reader)> {
        let range = args.range();
        self.inner
            .read(path, args)
            .map_ok(|(rp, r)| {
                (
                    rp,
                    ErrorContextWrapper::new(self.meta.scheme(), path.to_string(), r)
                        .with_range(range),
                )
            })
            .map_err(|err| {
                err.with_operation(Operation::Read)
                    .with_context("service", self.meta.scheme())
                    .with_context("path", path)
                    .with_context("range", range.to_string())
            })
            .await
    }

    async fn write(&self, path: &str, args: OpWrite) -> Result<(RpWrite, Self::Writer)> {
        self.inner
            .write(path, args)
            .map_ok(|(rp, w)| {
                (
                    rp,
                    ErrorContextWrapper::new(self.meta.scheme(), path.to_string(), w),
                )
            })
            .map_err(|err| {
                err.with_operation(Operation::Write)
                    .with_context("service", self.meta.scheme())
                    .with_context("path", path)
            })
            .await
    }

    async fn copy(&self, from: &str, to: &str, args: OpCopy) -> Result<RpCopy> {
        self.inner
            .copy(from, to, args)
            .map_err(|err| {
                err.with_operation(Operation::Copy)
                    .with_context("service", self.meta.scheme())
                    .with_context("from", from)
                    .with_context("to", to)
            })
            .await
    }

    async fn rename(&self, from: &str, to: &str, args: OpRename) -> Result<RpRename> {
        self.inner
            .rename(from, to, args)
            .map_err(|err| {
                err.with_operation(Operation::Rename)
                    .with_context("service", self.meta.scheme())
                    .with_context("from", from)
                    .with_context("to", to)
            })
            .await
    }

    async fn stat(&self, path: &str, args: OpStat) -> Result<RpStat> {
        self.inner
            .stat(path, args)
            .map_err(|err| {
                err.with_operation(Operation::Stat)
                    .with_context("service", self.meta.scheme())
                    .with_context("path", path)
            })
            .await
    }

    async fn delete(&self, path: &str, args: OpDelete) -> Result<RpDelete> {
        self.inner
            .delete(path, args)
            .map_err(|err| {
                err.with_operation(Operation::Delete)
                    .with_context("service", self.meta.scheme())
                    .with_context("path", path)
            })
            .await
    }

    async fn list(&self, path: &str, args: OpList) -> Result<(RpList, Self::Lister)> {
        self.inner
            .list(path, args)
            .map_ok(|(rp, p)| {
                (
                    rp,
                    ErrorContextWrapper::new(self.meta.scheme(), path.to_string(), p),
                )
            })
            .map_err(|err| {
                err.with_operation(Operation::List)
                    .with_context("service", self.meta.scheme())
                    .with_context("path", path)
            })
            .await
    }

    async fn batch(&self, args: OpBatch) -> Result<RpBatch> {
        self.inner
            .batch(args)
            .map_ok(|v| {
                let res = v
                    .into_results()
                    .into_iter()
                    .map(|(path, res)| {
                        let res = res.map_err(|err| {
                            err.with_operation(Operation::Delete)
                                .with_context("service", self.meta.scheme())
                                .with_context("path", &path)
                        });
                        (path, res)
                    })
                    .collect();

                RpBatch::new(res)
            })
            .map_err(|err| {
                err.with_operation(Operation::Batch)
                    .with_context("service", self.meta.scheme())
            })
            .await
    }

    async fn presign(&self, path: &str, args: OpPresign) -> Result<RpPresign> {
        self.inner.presign(path, args).await.map_err(|err| {
            err.with_operation(Operation::Presign)
                .with_context("service", self.meta.scheme())
                .with_context("path", path)
        })
    }

    fn blocking_create_dir(&self, path: &str, args: OpCreateDir) -> Result<RpCreateDir> {
        self.inner.blocking_create_dir(path, args).map_err(|err| {
            err.with_operation(Operation::BlockingCreateDir)
                .with_context("service", self.meta.scheme())
                .with_context("path", path)
        })
    }

    fn blocking_read(&self, path: &str, args: OpRead) -> Result<(RpRead, Self::BlockingReader)> {
        let range = args.range();
        self.inner
            .blocking_read(path, args)
            .map(|(rp, os)| {
                (
                    rp,
                    ErrorContextWrapper::new(self.meta.scheme(), path.to_string(), os)
                        .with_range(range),
                )
            })
            .map_err(|err| {
                err.with_operation(Operation::BlockingRead)
                    .with_context("service", self.meta.scheme())
                    .with_context("path", path)
                    .with_context("range", range.to_string())
            })
    }

    fn blocking_write(&self, path: &str, args: OpWrite) -> Result<(RpWrite, Self::BlockingWriter)> {
        self.inner
            .blocking_write(path, args)
            .map(|(rp, os)| {
                (
                    rp,
                    ErrorContextWrapper::new(self.meta.scheme(), path.to_string(), os),
                )
            })
            .map_err(|err| {
                err.with_operation(Operation::BlockingWrite)
                    .with_context("service", self.meta.scheme())
                    .with_context("path", path)
            })
    }

    fn blocking_copy(&self, from: &str, to: &str, args: OpCopy) -> Result<RpCopy> {
        self.inner.blocking_copy(from, to, args).map_err(|err| {
            err.with_operation(Operation::BlockingCopy)
                .with_context("service", self.meta.scheme())
                .with_context("from", from)
                .with_context("to", to)
        })
    }

    fn blocking_rename(&self, from: &str, to: &str, args: OpRename) -> Result<RpRename> {
        self.inner.blocking_rename(from, to, args).map_err(|err| {
            err.with_operation(Operation::BlockingRename)
                .with_context("service", self.meta.scheme())
                .with_context("from", from)
                .with_context("to", to)
        })
    }

    fn blocking_stat(&self, path: &str, args: OpStat) -> Result<RpStat> {
        self.inner.blocking_stat(path, args).map_err(|err| {
            err.with_operation(Operation::BlockingStat)
                .with_context("service", self.meta.scheme())
                .with_context("path", path)
        })
    }

    fn blocking_delete(&self, path: &str, args: OpDelete) -> Result<RpDelete> {
        self.inner.blocking_delete(path, args).map_err(|err| {
            err.with_operation(Operation::BlockingDelete)
                .with_context("service", self.meta.scheme())
                .with_context("path", path)
        })
    }

    fn blocking_list(&self, path: &str, args: OpList) -> Result<(RpList, Self::BlockingLister)> {
        self.inner
            .blocking_list(path, args)
            .map(|(rp, os)| {
                (
                    rp,
                    ErrorContextWrapper::new(self.meta.scheme(), path.to_string(), os),
                )
            })
            .map_err(|err| {
                err.with_operation(Operation::BlockingList)
                    .with_context("service", self.meta.scheme())
                    .with_context("path", path)
            })
    }
}

pub struct ErrorContextWrapper<T> {
    scheme: Scheme,
    path: String,
    inner: T,
    range: BytesRange,
    processed: u64,
}

impl<T> ErrorContextWrapper<T> {
    fn new(scheme: Scheme, path: String, inner: T) -> Self {
        Self {
            scheme,
            path,
            inner,
            range: BytesRange::default(),
            processed: 0,
        }
    }

    fn with_range(mut self, range: BytesRange) -> Self {
        self.range = range;
        self
    }
}

impl<T: oio::Read> oio::Read for ErrorContextWrapper<T> {
    async fn read(&mut self) -> Result<Buffer> {
        self.inner
            .read()
            .await
            .map(|bs| {
                self.processed += bs.len() as u64;
                bs
            })
            .map_err(|err| {
                err.with_operation(ReadOperation::Read)
                    .with_context("service", self.scheme)
                    .with_context("path", &self.path)
                    .with_context("range", self.range.to_string())
                    .with_context("read", self.processed.to_string())
            })
    }
}

impl<T: oio::BlockingRead> oio::BlockingRead for ErrorContextWrapper<T> {
    fn read(&mut self) -> Result<Buffer> {
        self.inner
            .read()
            .map(|bs| {
                self.processed += bs.len() as u64;
                bs
            })
            .map_err(|err| {
                err.with_operation(ReadOperation::BlockingRead)
                    .with_context("service", self.scheme)
                    .with_context("path", &self.path)
                    .with_context("range", self.range.to_string())
                    .with_context("read", self.processed.to_string())
            })
    }
}

impl<T: oio::Write> oio::Write for ErrorContextWrapper<T> {
    async fn write(&mut self, bs: Buffer) -> Result<()> {
        let size = bs.len();
        self.inner
            .write(bs)
            .await
            .map(|_| {
                self.processed += size as u64;
            })
            .map_err(|err| {
                err.with_operation(WriteOperation::Write)
                    .with_context("service", self.scheme)
                    .with_context("path", &self.path)
                    .with_context("size", size.to_string())
                    .with_context("written", self.processed.to_string())
            })
    }

    async fn close(&mut self) -> Result<()> {
        self.inner.close().await.map_err(|err| {
            err.with_operation(WriteOperation::Close)
                .with_context("service", self.scheme)
                .with_context("path", &self.path)
                .with_context("written", self.processed.to_string())
        })
    }

    async fn abort(&mut self) -> Result<()> {
        self.inner.abort().await.map_err(|err| {
            err.with_operation(WriteOperation::Abort)
                .with_context("service", self.scheme)
                .with_context("path", &self.path)
                .with_context("processed", self.processed.to_string())
        })
    }
}

impl<T: oio::BlockingWrite> oio::BlockingWrite for ErrorContextWrapper<T> {
    fn write(&mut self, bs: Buffer) -> Result<()> {
        let size = bs.len();
        self.inner
            .write(bs)
            .map(|_| {
                self.processed += size as u64;
            })
            .map_err(|err| {
                err.with_operation(WriteOperation::BlockingWrite)
                    .with_context("service", self.scheme)
                    .with_context("path", &self.path)
                    .with_context("size", size.to_string())
                    .with_context("written", self.processed.to_string())
            })
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close().map_err(|err| {
            err.with_operation(WriteOperation::BlockingClose)
                .with_context("service", self.scheme)
                .with_context("path", &self.path)
                .with_context("written", self.processed.to_string())
        })
    }
}

impl<T: oio::List> oio::List for ErrorContextWrapper<T> {
    async fn next(&mut self) -> Result<Option<oio::Entry>> {
        self.inner
            .next()
            .await
            .map(|bs| {
                self.processed += bs.is_some() as u64;
                bs
            })
            .map_err(|err| {
                err.with_operation(ListOperation::Next)
                    .with_context("service", self.scheme)
                    .with_context("path", &self.path)
                    .with_context("listed", self.processed.to_string())
            })
    }
}

impl<T: oio::BlockingList> oio::BlockingList for ErrorContextWrapper<T> {
    fn next(&mut self) -> Result<Option<oio::Entry>> {
        self.inner
            .next()
            .map(|bs| {
                self.processed += bs.is_some() as u64;
                bs
            })
            .map_err(|err| {
                err.with_operation(ListOperation::BlockingNext)
                    .with_context("service", self.scheme)
                    .with_context("path", &self.path)
                    .with_context("listed", self.processed.to_string())
            })
    }
}
