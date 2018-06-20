use comrak::{markdown_to_html, ComrakOptions};
use heck::KebabCase;
use rocket::request::Form;
use rocket::response::{Redirect, Flash};
use rocket_contrib::Template;
use serde_json;

use activity_pub::{broadcast, context, activity_pub, ActivityPub};
use db_conn::DbConn;
use models::{
    blogs::*,
    comments::Comment,
    post_authors::*,
    posts::*,
    users::User
};
use utils;
use safe_string::SafeString;

#[get("/~/<blog>/<slug>", rank = 4)]
fn details(blog: String, slug: String, conn: DbConn, user: Option<User>) -> Template {
    may_fail!(Blog::find_by_fqn(&*conn, blog), "Couldn't find this blog", |blog| {
        may_fail!(Post::find_by_slug(&*conn, slug, blog.id), "Couldn't find this post", |post| {
            let comments = Comment::find_by_post(&*conn, post.id);

            Template::render("posts/details", json!({
                "author": post.get_authors(&*conn)[0].to_json(&*conn),
                "post": post,
                "blog": blog,
                "comments": comments.into_iter().map(|c| c.to_json(&*conn)).collect::<Vec<serde_json::Value>>(),
                "n_likes": post.get_likes(&*conn).len(),
                "has_liked": user.clone().map(|u| u.has_liked(&*conn, &post)).unwrap_or(false),
                "n_reshares": post.get_reshares(&*conn).len(),
                "has_reshared": user.clone().map(|u| u.has_reshared(&*conn, &post)).unwrap_or(false),
                "account": user,
                "date": &post.creation_date.timestamp()
            }))
        })
    })
}

#[get("/~/<blog>/<slug>", rank = 3, format = "application/activity+json")]
fn activity_details(blog: String, slug: String, conn: DbConn) -> ActivityPub {
    let blog = Blog::find_by_fqn(&*conn, blog).unwrap();
    let post = Post::find_by_slug(&*conn, slug, blog.id).unwrap();

    let mut act = serde_json::to_value(post.into_activity(&*conn)).unwrap();
    act["@context"] = context();
    activity_pub(act)
}

#[get("/~/<blog>/new", rank = 2)]
fn new_auth(blog: String) -> Flash<Redirect> {
    utils::requires_login("You need to be logged in order to write a new post", uri!(new: blog = blog))
}

#[get("/~/<blog>/new", rank = 1)]
fn new(blog: String, user: User, conn: DbConn) -> Template {
    let b = Blog::find_by_fqn(&*conn, blog.to_string()).unwrap();

    if !user.is_author_in(&*conn, b.clone()) {
        Template::render("errors/403", json!({
            "error_message": "You are not author in this blog."
        }))
    } else {
        Template::render("posts/new", json!({
            "account": user
        }))
    }
}

#[derive(FromForm)]
struct NewPostForm {
    pub title: String,
    pub content: String,
    pub license: String
}

#[post("/~/<blog_name>/new", data = "<data>")]
fn create(blog_name: String, data: Form<NewPostForm>, user: User, conn: DbConn) -> Redirect {
    let blog = Blog::find_by_fqn(&*conn, blog_name.to_string()).unwrap();
    let form = data.get();
    let slug = form.title.to_string().to_kebab_case();

    if !user.is_author_in(&*conn, blog.clone()) {
        Redirect::to(uri!(super::blogs::details: name = blog_name))
    } else {
        if slug == "new" || Post::find_by_slug(&*conn, slug.clone(), blog.id).is_some() {
            Redirect::to(uri!(new: blog = blog_name))
        } else {
            let content = markdown_to_html(form.content.to_string().as_ref(), &ComrakOptions{
                smart: true,
                safe: true,
                ext_strikethrough: true,
                ext_tagfilter: true,
                ext_table: true,
                ext_autolink: true,
                ext_tasklist: true,
                ext_superscript: true,
                ext_header_ids: Some("title".to_string()),
                ext_footnotes: true,
                ..ComrakOptions::default()
            });

            let post = Post::insert(&*conn, NewPost {
                blog_id: blog.id,
                slug: slug.to_string(),
                title: form.title.to_string(),
                content: SafeString::new(&content),
                published: true,
                license: form.license.to_string(),
                ap_url: "".to_string()
            });
            post.update_ap_url(&*conn);
            PostAuthor::insert(&*conn, NewPostAuthor {
                post_id: post.id,
                author_id: user.id
            });

            let act = post.create_activity(&*conn);
            broadcast(&*conn, &user, act, user.get_followers(&*conn));

            Redirect::to(uri!(details: blog = blog_name, slug = slug))
        }
    }
}
