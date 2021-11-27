use std::{
	cell::RefCell,
	pin::Pin,
	rc::Rc,
	sync::Arc,
	task::{Context, Poll},
};

use actix_web::{
	dev::{Service, ServiceRequest, ServiceResponse, Transform},
	error::ErrorBadGateway,
	web::Data,
	Error, HttpMessage,
};
use actix_web_httpauth::extractors::{bearer, AuthExtractor, AuthenticationError};
use futures_util::{
	future::{ok, Ready},
	Future, FutureExt,
};

use super::openid::ValidationError;
use crate::objects::User;

pub struct BearerAuth<F, O>
where
	F: Fn(ServiceRequest, bearer::BearerAuth) -> O + 'static,
	O: Future<Output = Result<ServiceRequest, Error>> + 'static,
{
	function: Arc<F>,
}

impl<F, O> BearerAuth<F, O>
where
	F: Fn(ServiceRequest, bearer::BearerAuth) -> O + 'static,
	O: Future<Output = Result<ServiceRequest, Error>> + 'static,
{
	pub fn new(function: F) -> Self {
		Self {
			function: Arc::new(function),
		}
	}
}

impl<S, B, F, O> Transform<S> for BearerAuth<F, O>
where
	S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
	S::Future: 'static,
	B: 'static,
	F: Fn(ServiceRequest, bearer::BearerAuth) -> O + 'static,
	O: Future<Output = Result<ServiceRequest, Error>> + 'static,
{
	type Error = Error;
	type Future = Ready<Result<Self::Transform, Self::InitError>>;
	type InitError = ();
	type Request = ServiceRequest;
	type Response = ServiceResponse<B>;
	type Transform = BearerAuthMiddleware<S, B, F, O>;

	fn new_transform(
		&self,
		service: S,
	) -> Self::Future {
		ok(BearerAuthMiddleware {
			service: Rc::new(RefCell::new(service)),
			function: Arc::clone(&self.function),
		})
	}
}

pub struct BearerAuthMiddleware<S, B, F, O>
where
	S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
	S::Future: 'static,
	B: 'static,
	F: Fn(ServiceRequest, bearer::BearerAuth) -> O + 'static,
	O: Future<Output = Result<ServiceRequest, Error>> + 'static,
{
	service: Rc<RefCell<S>>,
	function: Arc<F>,
}

impl<S, B, F, O> Service for BearerAuthMiddleware<S, B, F, O>
where
	S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
	S::Future: 'static,
	B: 'static,
	F: Fn(ServiceRequest, bearer::BearerAuth) -> O + 'static,
	O: Future<Output = Result<ServiceRequest, Error>> + 'static,
{
	type Error = Error;
	type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;
	type Request = ServiceRequest;
	type Response = ServiceResponse<B>;

	fn poll_ready(
		&mut self,
		cx: &mut Context<'_>,
	) -> Poll<Result<(), Self::Error>> {
		self.service.poll_ready(cx)
	}

	fn call(
		&mut self,
		req: ServiceRequest,
	) -> Self::Future {
		let service = Rc::clone(&self.service);

		let function = Arc::clone(&self.function);

		async move {
			let res = match bearer::BearerAuth::from_service_request(&req).await {
				Ok(credentials) => function(req, credentials).await?,
				Err(_) => req,
			};
			// Ensure `borrow_mut()` and `.await` are on separate lines or else a panic
			// occurs.
			let fut = service.borrow_mut().call(res);
			fut.await
		}
		.boxed_local()
	}
}

pub async fn validator(
	request: ServiceRequest,
	credentials: bearer::BearerAuth,
) -> Result<ServiceRequest, Error> {
	let auth_config = request
		.app_data::<Data<bearer::Config>>()
		.map(|data| data.as_ref().clone())
		.unwrap_or_default();

	match crate::authentication::openid::validate_token(credentials.token()).await {
		Ok(token_data) => {
			request
				.extensions_mut()
				.insert(User::from(token_data.claims));
			Ok(request)
		},
		Err(ValidationError::DiscoveryError(_)) => {
			Err(ErrorBadGateway("failed to get ID provider data"))
		},
		Err(_) => Err(AuthenticationError::from(auth_config).into()),
	}
}
