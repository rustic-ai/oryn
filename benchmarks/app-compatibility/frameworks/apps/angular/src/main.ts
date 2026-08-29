import {Component,provideZonelessChangeDetection} from '@angular/core';import {bootstrapApplication} from '@angular/platform-browser';
@Component({selector:'compatibility-root',standalone:true,template:`<main data-framework="angular"><h1>Angular ready</h1><label>Search<input aria-label="Search" [value]="query" (input)="query=$any($event.target).value"></label><button (click)="add()">Add item</button><output>Items: {{count}}</output><a href="?route=details">Route</a></main>`})
class App{count=0;query='';add(){this.count++;localStorage.setItem('angular-count',String(this.count));}}
bootstrapApplication(App,{providers:[provideZonelessChangeDetection()]});
