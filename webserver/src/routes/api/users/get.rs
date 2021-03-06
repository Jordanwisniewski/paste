use crate::{
  config::Config,
  database::{
    DbConn,
    models::{
      pastes::Paste as DbPaste,
      users::User,
    },
    schema::{users, pastes},
  },
  models::{
    paste::{
      Visibility, Content,
      output::{Output, OutputAuthor},
    },
    status::{Status, ErrorKind},
  },
  routes::{OptionalUser, RouteResult},
};

use diesel::{prelude::*, dsl::count};

use rocket::{http::Status as HttpStatus, State};

use std::{fs::File, io::Read};

#[get("/<username>?<page>")]
pub fn get(username: String, page: Option<u32>, user: OptionalUser, conn: DbConn, config: State<Config>) -> RouteResult<Vec<Output>> {
  let page = page.unwrap_or(1);
  // TODO: make PositiveNumber struct or similar (could make Positive<num::Integer> or something)
  if page == 0 {
    return Ok(Status::show_error(
      HttpStatus::BadRequest,
      ErrorKind::BadParameters(Some("page number must be greater than 0".into())),
    ));
  }
  let target: User = match users::table.filter(users::username.eq(&username)).first(&*conn).optional()? {
    Some(u) => u,
    None => return Ok(Status::show_error(HttpStatus::NotFound, ErrorKind::MissingUser)),
  };

  let mut query = DbPaste::belonging_to(&target)
    .select(count(pastes::id))
    .into_boxed();
  if Some(target.id()) != user.as_ref().map(|x| x.id()) {
    query = query.filter(pastes::visibility.eq(Visibility::Public));
  }
  let total_pastes: i64 = query.get_result(&*conn)?;

  let outputs = if total_pastes == 0 && page == 1 {
    Vec::default()
  } else {
    let page = i64::from(page);
    let offset = (page - 1) * 15;
    if offset >= total_pastes {
      return Ok(Status::show_error(HttpStatus::NotFound, ErrorKind::MissingPaste));
    }
    let pastes: Vec<DbPaste> = if Some(target.id()) == user.as_ref().map(|x| x.id()) {
      DbPaste::belonging_to(&target)
        .order_by(pastes::created_at.desc())
        .offset(offset)
        .limit(15)
        .load(&*conn)?
    } else {
      DbPaste::belonging_to(&target)
        .filter(pastes::visibility.eq(Visibility::Public))
        .order_by(pastes::created_at.desc())
        .offset(offset)
        .limit(15)
        .load(&*conn)?
    };

    let author = OutputAuthor::new(target.id(), target.username(), target.name());

    let mut outputs = Vec::with_capacity(pastes.len());

    for paste in pastes {
      let id = paste.id();

      let files = id.files(&conn)?;
      let mut has_preview = false;

      let mut output_files = Vec::with_capacity(files.len());

      const LEN: usize = 257;
      let mut bytes = [0; LEN];

      for file in files {
        let mut f = file.as_output_file(&*config, false, &paste)?;

        // TODO: maybe store this in database or its own file?
        if !has_preview && file.is_binary() != Some(true) {
          let path = file.path(&*config, &paste);
          let read = File::open(path)?.read(&mut bytes)?;
          let full = read < LEN;
          let end = if read == LEN { read - 1 } else { read };

          let preview = match String::from_utf8(bytes[..end].to_vec()) {
            Ok(s) => Some((full, s)),
            Err(e) => {
              let valid = e.utf8_error().valid_up_to();
              if valid > 0 {
                let p = unsafe { String::from_utf8_unchecked(bytes[..valid].to_vec()) };
                Some((full, p))
              } else {
                None
              }
            },
          };

          if let Some((full, mut p)) = preview {
            if !full {
              if let Some((mut i, _)) = p.rmatch_indices(|x| x == '\r' || x == '\n').next() {
                if i != 0 && p.len() > i && &p[i - 1..i] == "\r" {
                  i -= 1;
                }
                p.truncate(i);
              }
            }

            f.content = Some(Content::Text(p));
            has_preview = true;
          }
        }

        output_files.push(f);
      }

      outputs.push(Output::new(
        paste.id(),
        Some(author.clone()),
        paste.name(),
        paste.description(),
        paste.visibility(),
        paste.created_at(),
        paste.updated_at(&*config).ok(), // FIXME
        paste.expires(),
        None,
        output_files,
      ));
    }

    outputs
  };

  Ok(Status::show_success(HttpStatus::Ok, outputs))
}
